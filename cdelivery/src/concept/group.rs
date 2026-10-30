use getset::Getters;

#[derive(Debug, Clone, PartialEq, Eq, Getters)]
pub(crate) struct Group {
    #[getset(get = "pub(crate)")]
    name: super::name::Name,
    #[getset(get = "pub(crate)")]
    description: Option<String>,
    #[getset(get = "pub(crate)")]
    network_id: String,
}

impl Group {
    pub(crate) fn new(
        name: super::name::Name,
        description: Option<String>,
        network_id: String,
    ) -> Self {
        Self {
            name,
            description: description.filter(|d| !d.is_empty()),
            network_id,
        }
    }

    pub(crate) fn set_description(&mut self, description: Option<String>) {
        self.description = description.filter(|d| !d.is_empty());
    }
}

impl super::versioned::Aggregate for Group {}
