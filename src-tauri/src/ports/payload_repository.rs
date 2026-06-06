use crate::domain::manifest::MirrorEndpoints;
use crate::errors::AppError;

pub trait PayloadRepository {
    fn endpoints(&self) -> &MirrorEndpoints;
    fn refresh_manifest(&self) -> Result<(), AppError>;
}
