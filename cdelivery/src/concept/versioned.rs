pub(crate) trait Aggregate: Clone + Eq {}

#[derive(Debug)]
pub(crate) struct Versioned<T> {
    version: u64,
    baseline: T,
    value: T,
}

impl<T: Aggregate> Versioned<T> {
    pub(crate) fn new(version: u64, value: T) -> Self {
        Self {
            version,
            baseline: value.clone(),
            value,
        }
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn get(&self) -> &T {
        &self.value
    }

    pub(crate) fn baseline(&self) -> &T {
        &self.baseline
    }

    pub(crate) fn get_mut(&mut self) -> &mut T {
        &mut self.value
    }

    pub(crate) fn changed(&self) -> bool {
        self.value != self.baseline
    }

    pub(crate) fn into_inner(self) -> T {
        self.value
    }
}
