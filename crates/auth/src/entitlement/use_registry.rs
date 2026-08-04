//! Registry of KR-derived uses and the Member-visible surfaces that consume them.
//!
//! Every Member-visible KR-derived surface (dataset, factor, recommendation,
//! backtest, report, benchmark, Paper view, payload, download) is declared here and
//! gates through the **same** [`crate::entitlement::service::EntitlementService`].
//! Owner-only development paths are also enumerated so the service can allow them
//! for the Owner without any entitlement.

/// A KR-derived use of market data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KrUse {
    // --- Member-visible surfaces (fail closed without ACTIVE entitlement) -------
    /// Dataset view/query (Member-visible curated data).
    Dataset,
    /// Factor snapshot for a Member surface.
    Factor,
    /// Strategy recommendation.
    Recommendation,
    /// Backtest run and results.
    Backtest,
    /// Research/performance report.
    Report,
    /// Benchmark comparison.
    Benchmark,
    /// Paper (virtual) account performance view.
    PaperView,
    /// Any API payload carrying KR-derived data.
    Payload,
    /// Artifact download (files derived from KR data).
    Download,

    // --- Owner-only development paths (no entitlement required for Owner) -------
    /// Raw ingestion into the Raw zone.
    DevIngest,
    /// Curation of Raw into Curated datasets.
    DevCurate,
    /// Factor/selector development runs.
    DevFactor,
    /// Development backtests.
    DevBacktest,
    /// Development report generation.
    DevReport,
}

impl KrUse {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dataset => "dataset",
            Self::Factor => "factor",
            Self::Recommendation => "recommendation",
            Self::Backtest => "backtest",
            Self::Report => "report",
            Self::Benchmark => "benchmark",
            Self::PaperView => "paper_view",
            Self::Payload => "payload",
            Self::Download => "download",
            Self::DevIngest => "dev_ingest",
            Self::DevCurate => "dev_curate",
            Self::DevFactor => "dev_factor",
            Self::DevBacktest => "dev_backtest",
            Self::DevReport => "dev_report",
        }
    }

    /// True for the nine Member-visible KR-derived surfaces.
    pub const fn is_member_visible(self) -> bool {
        matches!(
            self,
            Self::Dataset
                | Self::Factor
                | Self::Recommendation
                | Self::Backtest
                | Self::Report
                | Self::Benchmark
                | Self::PaperView
                | Self::Payload
                | Self::Download
        )
    }

    /// True for the five Owner-only development paths.
    pub const fn is_owner_development(self) -> bool {
        matches!(
            self,
            Self::DevIngest
                | Self::DevCurate
                | Self::DevFactor
                | Self::DevBacktest
                | Self::DevReport
        )
    }
}

impl std::fmt::Display for KrUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The layer that consumes a Member-visible surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Api,
    Scheduler,
    Report,
    Artifact,
}

impl Layer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Scheduler => "scheduler",
            Self::Report => "report",
            Self::Artifact => "artifact",
        }
    }
}

/// Compile-time registry of the Member-visible KR surfaces. Each surface maps to a
/// [`KrUse`] and a consuming layer, and each is gated by the same service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KrMemberSurface {
    DatasetQuery,
    FactorView,
    Recommendation,
    BacktestRun,
    ReportView,
    BenchmarkView,
    PaperView,
    ApiPayload,
    ArtifactDownload,
}

impl KrMemberSurface {
    /// The nine Member-visible surfaces, in registry order.
    pub const ALL: [KrMemberSurface; 9] = [
        Self::DatasetQuery,
        Self::FactorView,
        Self::Recommendation,
        Self::BacktestRun,
        Self::ReportView,
        Self::BenchmarkView,
        Self::PaperView,
        Self::ApiPayload,
        Self::ArtifactDownload,
    ];

    pub const fn use_kind(self) -> KrUse {
        match self {
            Self::DatasetQuery => KrUse::Dataset,
            Self::FactorView => KrUse::Factor,
            Self::Recommendation => KrUse::Recommendation,
            Self::BacktestRun => KrUse::Backtest,
            Self::ReportView => KrUse::Report,
            Self::BenchmarkView => KrUse::Benchmark,
            Self::PaperView => KrUse::PaperView,
            Self::ApiPayload => KrUse::Payload,
            Self::ArtifactDownload => KrUse::Download,
        }
    }

    pub const fn layer(self) -> Layer {
        match self {
            Self::DatasetQuery | Self::FactorView | Self::PaperView | Self::ApiPayload => {
                Layer::Api
            }
            Self::Recommendation | Self::BacktestRun => Layer::Scheduler,
            Self::ReportView | Self::BenchmarkView => Layer::Report,
            Self::ArtifactDownload => Layer::Artifact,
        }
    }
}

/// The application-policy registry: the full enumeration of uses. Later OpenAPI
/// routes must resolve through [`KrMemberSurface`] so they share this gate.
#[derive(Debug, Clone, Copy)]
pub struct KrUseRegistry {
    member_visible: [KrUse; 9],
    owner_development: [KrUse; 5],
}

impl KrUseRegistry {
    /// The canonical registry.
    pub const fn standard() -> Self {
        Self {
            member_visible: [
                KrUse::Dataset,
                KrUse::Factor,
                KrUse::Recommendation,
                KrUse::Backtest,
                KrUse::Report,
                KrUse::Benchmark,
                KrUse::PaperView,
                KrUse::Payload,
                KrUse::Download,
            ],
            owner_development: [
                KrUse::DevIngest,
                KrUse::DevCurate,
                KrUse::DevFactor,
                KrUse::DevBacktest,
                KrUse::DevReport,
            ],
        }
    }

    pub const fn member_visible(&self) -> &[KrUse; 9] {
        &self.member_visible
    }

    pub const fn owner_development(&self) -> &[KrUse; 5] {
        &self.owner_development
    }

    /// Whether `use_kind` is a registered use.
    pub fn contains(&self, use_kind: KrUse) -> bool {
        self.member_visible.contains(&use_kind) || self.owner_development.contains(&use_kind)
    }

    /// The Member-visible surface corresponding to `use_kind`, if any.
    pub fn surface_for(&self, use_kind: KrUse) -> Option<KrMemberSurface> {
        KrMemberSurface::ALL
            .iter()
            .copied()
            .find(|s| s.use_kind() == use_kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_registry_is_complete() {
        let r = KrUseRegistry::standard();
        assert_eq!(r.member_visible().len(), 9);
        assert_eq!(r.owner_development().len(), 5);
        for use_kind in r.member_visible() {
            assert!(use_kind.is_member_visible());
            assert!(!use_kind.is_owner_development());
            assert!(r.contains(*use_kind));
        }
        for use_kind in r.owner_development() {
            assert!(use_kind.is_owner_development());
            assert!(!use_kind.is_member_visible());
            assert!(r.contains(*use_kind));
        }
    }

    #[test]
    fn every_member_surface_maps_to_a_registered_use() {
        let r = KrUseRegistry::standard();
        for surface in KrMemberSurface::ALL {
            let use_kind = surface.use_kind();
            assert!(
                use_kind.is_member_visible(),
                "{surface:?} must be member-visible"
            );
            assert!(r.contains(use_kind), "{surface:?} must be registered");
            assert_eq!(r.surface_for(use_kind), Some(surface));
        }
        assert_eq!(KrMemberSurface::ALL.len(), 9);
    }

    #[test]
    fn stable_use_tags() {
        assert_eq!(KrUse::PaperView.as_str(), "paper_view");
        assert_eq!(KrUse::Download.as_str(), "download");
        assert_eq!(KrUse::DevBacktest.as_str(), "dev_backtest");
        assert_eq!(Layer::Scheduler.as_str(), "scheduler");
    }
}
