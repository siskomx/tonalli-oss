use super::pack_boundary;
use crate::{
    pack::PackError,
    update::{app_version, current_platform, Store},
};
use std::path::Path;

/// A downloaded update whose signature, size and SHA-256 were all verified just now.
#[derive(uniffi::Record)]
pub struct UpdateReady {
    pub path: String,
    pub version: String,
    pub bytes: u64,
}

/// The staged package for this device, or nothing if there is no verified one. Called again by the
/// installer hand-off every time the file is read, so only verified bytes are ever handed out.
#[uniffi::export]
pub fn update_ready(data_dir: String) -> Result<Option<UpdateReady>, PackError> {
    pack_boundary(|| {
        let Some(platform) = current_platform() else {
            return Ok(None);
        };
        let ready = Store::new(Path::new(&data_dir)).ready(&app_version()?, platform)?;
        ready
            .map(|ready| {
                Ok(UpdateReady {
                    path: ready
                        .path
                        .to_str()
                        .ok_or_else(|| PackError::Corrupt("update path is not valid text".into()))?
                        .to_owned(),
                    version: ready.version,
                    bytes: ready.asset.bytes,
                })
            })
            .transpose()
    })
}
