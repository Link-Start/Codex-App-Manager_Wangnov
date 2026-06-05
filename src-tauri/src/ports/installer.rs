use crate::domain::operations::{OperationKind, OperationPlan};
use crate::domain::settings::AppSettings;
use crate::domain::target::Target;
use crate::errors::AppError;

pub trait Installer {
    fn plan(
        &self,
        kind: OperationKind,
        target: &Target,
        settings: &AppSettings,
    ) -> Result<OperationPlan, AppError>;
}

