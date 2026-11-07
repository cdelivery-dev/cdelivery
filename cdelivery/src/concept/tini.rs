pub(crate) const VERSION: &str = include_str!("../../../tini/VERSION");
pub(crate) const ENV_DIRECTORY: &str = "/run/cdelivery/env/";
pub(crate) const ARCH_ENV: &str = "CD_INIT_ARCH";
pub(crate) const AMD64_PATH: &str = "/run/cdelivery/tini/amd64";
pub(crate) const ARM64_PATH: &str = "/run/cdelivery/tini/arm64";
pub(crate) const INIT_PATH: &str = "/sbin/docker-init";

pub(crate) fn reserved_target(target: &str) -> bool {
    let target = std::path::Path::new(target);
    let internal = std::path::Path::new("/run/cdelivery");
    let init = std::path::Path::new(INIT_PATH);
    target.starts_with(internal)
        || internal.starts_with(target)
        || target.starts_with(init)
        || init.starts_with(target)
}
