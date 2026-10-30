use getset::{CopyGetters, Getters};

#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, macros_derive::Ctor)]
#[ctor(hydrate)]
pub(crate) struct Pingress {
    #[getset(get = "pub(crate)")]
    image: super::image::Reference,
    #[getset(get = "pub(crate)")]
    placement: super::node::Selector,
    #[getset(get_copy = "pub(crate)")]
    insight_logs: bool,
}

impl Pingress {
    pub(crate) fn retarget(
        &mut self,
        image: Option<super::image::Reference>,
        placement: Option<super::node::Selector>,
    ) {
        if let Some(image) = image {
            self.image = image;
        }
        if let Some(placement) = placement {
            self.placement = placement;
        }
    }

    pub(crate) fn set_insight_logs(&mut self, on: bool) {
        self.insight_logs = on;
    }
}

impl super::versioned::Aggregate for Pingress {}

#[derive(Debug, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Deployment {
    #[getset(get = "pub(crate)")]
    config: Pingress,
    #[getset(get_copy = "pub(crate)")]
    instances: u64,
    #[getset(get_copy = "pub(crate)")]
    acme_enabled: bool,
}

#[expect(clippy::cast_precision_loss)]
pub(crate) fn rate(count: u64, width: jiff::SignedDuration) -> f64 {
    let secs = width.as_secs_f64();
    if secs > 0.0 { count as f64 / secs } else { 0.0 }
}
