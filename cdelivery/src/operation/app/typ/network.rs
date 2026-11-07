use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    PortProtocol both concept::app::PortProtocol { Tcp, Udp }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Port {
    #[garde(range(min = 1))]
    pub(crate) target: u16,
    #[garde(range(min = 1))]
    pub(crate) published: u16,
    #[garde(skip)]
    pub(crate) protocol: PortProtocol,
}

super::super::super::typ::mirror_struct! {
    Port(p: Port) <-> concept::app::Port(c: concept::app::Port) {
        target: p.target => c.target(),
        published: p.published => c.published(),
        protocol: p.protocol.into() => c.protocol().into(),
    }
}
