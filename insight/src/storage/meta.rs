use std::collections::{BTreeMap, HashMap, HashSet};

use getset::{CopyGetters, Getters};
use object_store::path::Path;

pub const TERMINAL_LEVEL: u8 = 2;
pub const MERGE_L0_BATCH: usize = 8;
pub const MERGE_L1_TARGET: usize = 7;

#[expect(clippy::cast_possible_truncation)]
pub const ROLL_RECENT_WINDOW_NANOS: i64 = jiff::SignedDuration::from_hours(24).as_nanos() as i64;

pub fn temp_path(index_id: &str, node_id: &str, suffix: &str) -> Path {
    Path::from(format!(
        "{index_id}/{node_id}/{}{suffix}",
        uuid::Uuid::new_v4().simple()
    ))
}

#[derive(Debug, Clone, Copy)]
pub enum Site {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkStats {
    pub sealed: i64,
    pub min_timestamp_nanos: i64,
    pub max_timestamp_nanos: i64,
    pub rows: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Getters, CopyGetters)]
pub struct Chunk {
    node_id: String,
    #[getset(get_copy = "pub")]
    id: uuid::Uuid,
    #[getset(get_copy = "pub")]
    level: u8,
    stats: ChunkStats,
    #[getset(get = "pub")]
    path: Path,
}

impl Chunk {
    fn build(
        index_id: &str,
        node_id: String,
        id: uuid::Uuid,
        level: u8,
        stats: ChunkStats,
    ) -> Self {
        let path = Path::from(format!(
            "{index_id}/{node_id}/{}.{}.{}.{level}.{}.{}.parquet",
            id.simple(),
            stats.min_timestamp_nanos,
            stats.max_timestamp_nanos,
            stats.sealed,
            stats.rows,
        ));
        Self {
            node_id,
            id,
            level,
            stats,
            path,
        }
    }

    pub fn new(index_id: &str, node_id: &str, level: u8, stats: ChunkStats) -> Self {
        Self::build(
            index_id,
            node_id.to_owned(),
            uuid::Uuid::new_v4(),
            level,
            stats,
        )
    }

    #[must_use]
    pub fn next_generation(&self, index_id: &str, stats: ChunkStats) -> Self {
        Self::build(index_id, self.node_id.clone(), self.id, self.level, stats)
    }

    pub fn parse(path: &Path, size: u64) -> Option<(String, Self)> {
        let mut parts = path.as_ref().split_terminator('/');
        let owner = parts.next()?;
        let category = parts.next()?;
        let node_id = parts.next()?.to_owned();
        let file = parts.next()?;
        if parts.next().is_some() || owner.is_empty() || category.is_empty() || node_id.is_empty() {
            return None;
        }
        let index_id = format!("{owner}/{category}");
        let mut pieces = file.split('.');
        let id_hex = pieces.next()?;
        if id_hex.len() != 32 {
            return None;
        }
        let id = uuid::Uuid::try_parse(id_hex).ok()?;
        let min_timestamp_nanos = pieces.next()?.parse::<i64>().ok()?;
        let max_timestamp_nanos = pieces.next()?.parse::<i64>().ok()?;
        let level = pieces.next()?.parse::<u8>().ok()?;
        let sealed = pieces.next()?.parse::<i64>().ok()?;
        let rows = pieces.next()?.parse::<u64>().ok()?;
        if pieces.next()? != "parquet"
            || pieces.next().is_some()
            || min_timestamp_nanos > max_timestamp_nanos
        {
            return None;
        }
        Some((
            index_id,
            Self {
                node_id,
                id,
                level,
                stats: ChunkStats {
                    sealed,
                    min_timestamp_nanos,
                    max_timestamp_nanos,
                    rows,
                    size,
                },
                path: path.clone(),
            },
        ))
    }

    pub fn generation_key(&self) -> (u64, i64, &str) {
        (self.stats.rows, self.stats.sealed, self.path.as_ref())
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }
    pub fn stats(&self) -> ChunkStats {
        self.stats
    }
    pub fn time_span_nanos(&self) -> i64 {
        self.stats
            .max_timestamp_nanos
            .saturating_sub(self.stats.min_timestamp_nanos)
    }
    pub fn overlaps(&self, start_nanos: i64, end_nanos: i64) -> bool {
        self.stats.min_timestamp_nanos <= end_nanos && self.stats.max_timestamp_nanos >= start_nanos
    }
}

fn push_stale(stales: &mut Vec<Path>, path: Path) {
    if !stales.contains(&path) {
        stales.push(path);
    }
}

#[derive(Debug, Default)]
pub struct Record {
    local: Option<Chunk>,
    remote: Option<Chunk>,
    stale_local: Vec<Path>,
    stale_remote: Vec<Path>,
}

impl Record {
    fn slot_mut(&mut self, site: Site) -> (&mut Option<Chunk>, &mut Vec<Path>) {
        match site {
            Site::Local => (&mut self.local, &mut self.stale_local),
            Site::Remote => (&mut self.remote, &mut self.stale_remote),
        }
    }

    fn stales(&self, site: Site) -> &[Path] {
        match site {
            Site::Local => &self.stale_local,
            Site::Remote => &self.stale_remote,
        }
    }

    pub fn current(&self) -> Option<&Chunk> {
        match (&self.local, &self.remote) {
            (Some(l), Some(r)) => Some(if l.generation_key() >= r.generation_key() {
                l
            } else {
                r
            }),
            (Some(l), None) => Some(l),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        }
    }

    pub fn current_is_local(&self) -> bool {
        match (&self.local, &self.remote) {
            (Some(l), Some(r)) => l.generation_key() >= r.generation_key(),
            (Some(_), None) => true,
            _ => false,
        }
    }

    fn is_empty(&self) -> bool {
        self.local.is_none()
            && self.remote.is_none()
            && self.stale_local.is_empty()
            && self.stale_remote.is_empty()
    }
}

#[derive(Debug)]
pub struct Group {
    node_id: String,
    records: HashMap<uuid::Uuid, Record>,
    max_sealed_seen: i64,
}

impl Group {
    pub fn new(node_id: String) -> Self {
        Self {
            node_id,
            records: HashMap::new(),
            max_sealed_seen: 0,
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn next_sealed(&self, now_nanos: i64) -> i64 {
        now_nanos.max(self.max_sealed_seen.saturating_add(1))
    }

    pub fn record_at(&mut self, site: Site, chunk: Chunk) {
        self.max_sealed_seen = self.max_sealed_seen.max(chunk.stats().sealed);
        let record = self.records.entry(chunk.id()).or_default();
        let (slot, stales) = record.slot_mut(site);
        match slot {
            Some(current) if current.path == chunk.path => {}
            Some(current) if chunk.generation_key() > current.generation_key() => {
                if let Some(old) = slot.replace(chunk) {
                    push_stale(stales, old.path);
                }
            }
            Some(_) => push_stale(stales, chunk.path),
            None => *slot = Some(chunk),
        }
    }

    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.records.values()
    }

    pub fn visible(&self) -> Vec<&Chunk> {
        shadow_visible(self.records.values().filter_map(Record::current))
    }

    pub fn roll_candidate(&self, now_nanos: i64) -> Option<&Chunk> {
        let recent_threshold = now_nanos.saturating_sub(ROLL_RECENT_WINDOW_NANOS);
        self.visible()
            .into_iter()
            .filter(|c| c.level() == 0 && c.stats().max_timestamp_nanos > recent_threshold)
            .max_by_key(|c| c.stats().sealed)
    }

    fn retire_shadowed_at(&mut self, site: Site) {
        let candidates: Vec<&Chunk> = match site {
            Site::Local => self.records.values().filter_map(Record::current).collect(),
            Site::Remote => self
                .records
                .values()
                .filter_map(|r| r.remote.as_ref())
                .collect(),
        };
        let visible: HashSet<uuid::Uuid> = shadow_visible(candidates.iter().copied())
            .into_iter()
            .map(Chunk::id)
            .collect();
        let shadowed: Vec<uuid::Uuid> = candidates
            .into_iter()
            .filter(|c| !visible.contains(&c.id()))
            .map(Chunk::id)
            .collect();
        for id in shadowed {
            if let Some(record) = self.records.get_mut(&id) {
                let (slot, stales) = record.slot_mut(site);
                if let Some(taken) = slot.take() {
                    push_stale(stales, taken.path);
                }
            }
        }
    }

    pub fn retire_shadowed(&mut self) {
        self.retire_shadowed_at(Site::Local);
        self.retire_shadowed_at(Site::Remote);
    }

    pub fn retire_covered_local(&mut self, own_group: bool) {
        for record in self.records.values_mut() {
            let covered = match (&record.local, &record.remote) {
                (Some(local), Some(remote)) => {
                    remote.generation_key() >= local.generation_key()
                        && (!own_group || local.level() == TERMINAL_LEVEL)
                }
                _ => false,
            };
            if covered {
                let (slot, stales) = record.slot_mut(Site::Local);
                if let Some(local) = slot.take() {
                    push_stale(stales, local.path);
                }
            }
        }
    }

    pub fn retire_expired(&mut self, expired_before_nanos: i64) {
        for record in self.records.values_mut() {
            if record
                .current()
                .is_some_and(|c| c.stats().max_timestamp_nanos < expired_before_nanos)
            {
                for site in [Site::Local, Site::Remote] {
                    let (slot, stales) = record.slot_mut(site);
                    if let Some(taken) = slot.take() {
                        push_stale(stales, taken.path);
                    }
                }
            }
        }
    }

    pub fn stale_paths(&self, site: Site) -> impl Iterator<Item = (uuid::Uuid, &Path)> {
        self.records
            .iter()
            .flat_map(move |(id, r)| r.stales(site).iter().map(move |p| (*id, p)))
    }

    pub fn ack_stale(&mut self, site: Site, id: &uuid::Uuid, path: &Path) {
        if let Some(record) = self.records.get_mut(id) {
            let (_, stales) = record.slot_mut(site);
            stales.retain(|p| p != path);
        }
    }

    pub fn pull_candidates(&self) -> Vec<&Chunk> {
        let visible_ids: HashSet<uuid::Uuid> = self
            .visible()
            .into_iter()
            .filter(|c| c.level() < TERMINAL_LEVEL)
            .map(Chunk::id)
            .collect();
        self.records
            .iter()
            .filter(|(id, r)| visible_ids.contains(id) && r.local.is_none())
            .filter_map(|(_, r)| r.remote.as_ref())
            .collect()
    }

    pub fn resync_candidates(&self, expired_before_nanos: i64) -> Vec<&Chunk> {
        self.records
            .values()
            .filter_map(|record| {
                let local = record.local.as_ref()?;
                if local.stats().max_timestamp_nanos < expired_before_nanos {
                    return None;
                }
                match &record.remote {
                    Some(remote) if remote.generation_key() >= local.generation_key() => None,
                    _ => Some(local),
                }
            })
            .collect()
    }

    pub fn compact(&mut self) {
        self.records.retain(|_, r| !r.is_empty());
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

pub fn shadow_visible<'a, I>(chunks: I) -> Vec<&'a Chunk>
where
    I: IntoIterator<Item = &'a Chunk>,
{
    let mut by_level: BTreeMap<u8, Vec<&Chunk>> = BTreeMap::new();
    for chunk in chunks {
        by_level.entry(chunk.level()).or_default().push(chunk);
    }
    let mut horizon = i64::MIN;
    let mut out = Vec::new();
    for (_, level_chunks) in by_level.into_iter().rev() {
        let level_max = level_chunks.iter().map(|c| c.stats().sealed).max();
        for chunk in level_chunks {
            if chunk.stats().sealed > horizon {
                out.push(chunk);
            }
        }
        if let Some(level_max) = level_max {
            horizon = horizon.max(level_max);
        }
    }
    out
}

#[derive(Debug)]
pub struct MergeSelection {
    pub inputs: Vec<Chunk>,
    pub output_level: u8,
}

pub fn select_merge(
    visible: &[&Chunk],
    roll: Option<uuid::Uuid>,
    max_output_span_nanos: i64,
) -> Option<MergeSelection> {
    let mut l0: Vec<&Chunk> = visible
        .iter()
        .copied()
        .filter(|c| c.level() == 0 && Some(c.id()) != roll)
        .collect();
    let mut l1: Vec<&Chunk> = visible.iter().copied().filter(|c| c.level() == 1).collect();
    l0.sort_unstable_by_key(|c| c.stats().sealed);
    l1.sort_unstable_by_key(|c| c.stats().sealed);

    let (inputs, output_level): (Vec<&Chunk>, u8) = if l0.len() >= MERGE_L0_BATCH {
        if l1.len() >= MERGE_L1_TARGET {
            (
                l1.iter()
                    .chain(l0.iter().take(MERGE_L0_BATCH))
                    .copied()
                    .collect(),
                TERMINAL_LEVEL,
            )
        } else {
            (l0.iter().take(MERGE_L0_BATCH).copied().collect(), 1)
        }
    } else {
        return None;
    };

    let span_max = inputs.iter().map(|c| c.stats().max_timestamp_nanos).max()?;
    let span_min = inputs.iter().map(|c| c.stats().min_timestamp_nanos).min()?;
    if span_max.saturating_sub(span_min) > max_output_span_nanos {
        return None;
    }

    let output_sealed = inputs.iter().map(|c| c.stats().sealed).max()?;
    let selected: HashSet<uuid::Uuid> = inputs.iter().map(|c| c.id()).collect();
    let violates = visible.iter().any(|c| {
        c.level() < output_level && !selected.contains(&c.id()) && c.stats().sealed <= output_sealed
    });
    if violates {
        return None;
    }

    Some(MergeSelection {
        inputs: inputs.into_iter().cloned().collect(),
        output_level,
    })
}

#[derive(Debug, Default)]
pub struct Index {
    groups: HashMap<String, Group>,
}

impl Index {
    pub fn group(&self, node_id: &str) -> Option<&Group> {
        self.groups.get(node_id)
    }

    pub fn group_mut(&mut self, node_id: &str) -> &mut Group {
        self.groups
            .entry(node_id.to_owned())
            .or_insert_with(|| Group::new(node_id.to_owned()))
    }

    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.groups.values()
    }

    pub fn groups_mut(&mut self) -> impl Iterator<Item = &mut Group> {
        self.groups.values_mut()
    }

    pub fn record_at(&mut self, site: Site, chunk: Chunk) {
        let node_id = chunk.node_id.clone();
        self.group_mut(&node_id).record_at(site, chunk);
    }

    pub fn compact(&mut self) {
        for group in self.groups.values_mut() {
            group.compact();
        }
        self.groups.retain(|_, g| !g.is_empty());
    }
}
