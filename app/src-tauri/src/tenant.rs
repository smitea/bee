#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantAccess {
    pub tenant: u16,
}

impl TenantAccess {
    pub const GLOBAL: TenantAccess = TenantAccess { tenant: 0 };

    pub fn new(tenant: u16) -> Self {
        Self { tenant }
    }

    pub fn is_global(&self) -> bool {
        self.tenant == 0
    }
}

pub fn can_access_datasource(job_tenant: u16, ds_tenant: u16) -> bool {
    ds_tenant == 0 || ds_tenant == job_tenant
}

pub fn validate_tenant(value: u16) -> Result<u16, String> {
    if value > u16::MAX {
        return Err(format!(
            "tenant: value {value} out of range 0..={}",
            u16::MAX
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_job_only_accesses_global_datasources() {
        assert!(can_access_datasource(0, 0));
        assert!(!can_access_datasource(0, 1));
        assert!(!can_access_datasource(0, 65535));
    }

    #[test]
    fn tenant_zero_datasource_matches_any_job_tenant() {
        assert!(can_access_datasource(1, 0));
        assert!(can_access_datasource(42, 0));
        assert!(can_access_datasource(65535, 0));
    }

    #[test]
    fn matching_tenant_allows_access() {
        assert!(can_access_datasource(1, 1));
        assert!(can_access_datasource(42, 42));
        assert!(can_access_datasource(65535, 65535));
    }

    #[test]
    fn mismatching_tenants_reject_access() {
        assert!(!can_access_datasource(1, 2));
        assert!(!can_access_datasource(2, 1));
        assert!(!can_access_datasource(100, 200));
    }

    #[test]
    fn validate_tenant_accepts_zero_and_max() {
        assert_eq!(validate_tenant(0).unwrap(), 0);
        assert_eq!(validate_tenant(u16::MAX).unwrap(), u16::MAX);
    }

    #[test]
    fn validate_tenant_accepts_arbitrary_in_range() {
        assert_eq!(validate_tenant(1).unwrap(), 1);
        assert_eq!(validate_tenant(42).unwrap(), 42);
        assert_eq!(validate_tenant(65534).unwrap(), 65534);
    }

    #[test]
    fn tenant_access_is_global_when_zero() {
        assert!(TenantAccess::new(0).is_global());
        assert!(TenantAccess::GLOBAL.is_global());
        assert!(!TenantAccess::new(1).is_global());
        assert!(!TenantAccess::new(42).is_global());
    }

    #[test]
    fn tenant_access_can_be_compared() {
        assert_eq!(TenantAccess::new(5), TenantAccess::new(5));
        assert_ne!(TenantAccess::new(5), TenantAccess::new(6));
    }
}