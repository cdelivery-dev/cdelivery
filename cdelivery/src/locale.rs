use std::cell::Cell;

#[derive(
    utoipa::ToSchema, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub(crate) enum Locale {
    De,
    En,
    Es,
    Fr,
    Ja,
    PtBr,
    ZhCn,
    ZhTw,
}

impl Locale {
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
            Self::Es => "es",
            Self::Fr => "fr",
            Self::Ja => "ja",
            Self::PtBr => "pt-BR",
            Self::ZhCn => "zh-CN",
            Self::ZhTw => "zh-TW",
        }
    }

    pub(crate) fn quantity<T>(self, value: u64, one: T, other: T) -> T {
        let singular = match self {
            Self::De | Self::En | Self::Es => value == 1,
            Self::Fr | Self::PtBr => value <= 1,
            Self::Ja | Self::ZhCn | Self::ZhTw => false,
        };
        if singular { one } else { other }
    }
}

tokio::task_local! {
    pub(crate) static LOCALE: Cell<Locale>;
}

pub(crate) fn current() -> Locale {
    LOCALE.try_with(Cell::get).unwrap_or(Locale::En)
}

pub(crate) fn negotiate(header: Option<&str>) -> Locale {
    header
        .into_iter()
        .flat_map(|h| h.split(','))
        .filter_map(candidate)
        .fold(None, |best: Option<(f32, Locale)>, cand| match best {
            Some((q, _)) if q >= cand.0 => best,
            _ => Some(cand),
        })
        .map_or(Locale::En, |(_, locale)| locale)
}

fn candidate(part: &str) -> Option<(f32, Locale)> {
    let mut parts = part.split(';');
    let tag = parts.next()?.trim().to_ascii_lowercase();
    let q = parts.next().map_or(Some(1.0), |param| {
        quality(param.trim().strip_prefix(['q', 'Q'])?.strip_prefix('=')?)
    })?;
    if parts.next().is_some() || q <= 0.0 {
        return None;
    }
    let mut subtags = tag.split('-');
    let locale = match subtags.next()? {
        "de" => Locale::De,
        "en" | "*" => Locale::En,
        "es" => Locale::Es,
        "fr" => Locale::Fr,
        "ja" => Locale::Ja,
        "pt" => Locale::PtBr,
        "zh" => chinese(subtags),
        _ => return None,
    };
    Some((q, locale))
}

fn chinese(subtags: std::str::Split<'_, char>) -> Locale {
    let mut locale = Locale::ZhCn;
    for subtag in subtags.take_while(|subtag| subtag.len() != 1) {
        match subtag {
            "hans" => return Locale::ZhCn,
            "hant" => return Locale::ZhTw,
            "hk" | "mo" | "tw" => locale = Locale::ZhTw,
            _ => {}
        }
    }
    locale
}

fn quality(value: &str) -> Option<f32> {
    let value = value.trim();
    let (int, frac) = value.split_once('.').unwrap_or((value, ""));
    if !matches!(int, "0" | "1") || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|q| (0.0..=1.0).contains(q))
}

macro_rules! tt {
    (locale = $locale:expr, $key:expr $(, $n:ident = $v:expr)* $(,)?) => {
        rust_i18n::t!($key, locale = ($locale).tag() $(, $n = $v)*)
    };
    ($key:expr $(, $n:ident = $v:expr)* $(,)?) => {
        rust_i18n::t!($key, locale = crate::locale::current().tag() $(, $n = $v)*)
    };
}
pub(crate) use tt;
