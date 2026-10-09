//! Bounded, deterministic recovery recommendations from sealed acquisition maps.
//! Scores are scheduling heuristics, not probabilities or recovered-file claims.
use serde::{Deserialize, Serialize};

pub const POLICY: &str = "usb-fast-pass-priority-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbRoute {
    Complete,
    MoveToGreaseweazle,
}

/// Persisted with the USB receipt. No source bytes are synthesized or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsbTriage {
    pub policy: String,
    pub route: UsbRoute,
    pub attempt: u32,
    pub image_sha256: String,
    pub total_sectors: usize,
    pub missing_sectors: usize,
    pub saved_generation: u64,
}

impl UsbTriage {
    pub(crate) fn new(
        attempt: u32,
        image_sha256: String,
        total_sectors: usize,
        missing_sectors: usize,
        saved_generation: u64,
    ) -> Self {
        Self {
            policy: POLICY.into(),
            route: if missing_sectors == 0 {
                UsbRoute::Complete
            } else {
                UsbRoute::MoveToGreaseweazle
            },
            attempt,
            image_sha256,
            total_sectors,
            missing_sectors,
            saved_generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Ready,
    HeldInUsb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Few,
    Localized,
    Moderate,
    Severe,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub disk: u32,
    pub attempt: u32,
    pub image_sha256: String,
    pub availability: Availability,
    pub severity: Severity,
    pub missing_sectors: usize,
    pub total_sectors: usize,
    pub boot_sector_missing: bool,
    pub waiting_generations: Option<u64>,
    pub score: u32,
    pub reason: String,
}

pub(crate) struct Candidate<'a> {
    pub disk: u32,
    pub attempt: u32,
    pub image_sha256: &'a str,
    pub total_sectors: usize,
    pub bad: &'a [u64],
    pub availability: Availability,
    pub saved_generation: Option<u64>,
}

/// Availability dominates score: never recommend a still-held USB disk ahead
/// of removal-confirmed work. Active/finished GW work is excluded by the caller.
pub(crate) fn rank(candidates: Vec<Candidate<'_>>, generation: u64) -> Vec<Recommendation> {
    let mut ranked = candidates
        .into_iter()
        .map(|c| {
            let missing = c.bad.len();
            let (severity, base, cost) = if missing <= 2 {
                (Severity::Few, 500, "few missing sectors")
            } else if missing <= 18 {
                (Severity::Localized, 400, "localized missing-sector count")
            } else if missing <= c.total_sectors / 10 {
                (Severity::Moderate, 250, "moderate missing-sector count")
            } else {
                (Severity::Severe, 100, "severe missing-sector count")
            };
            let boot_sector_missing = c.bad.contains(&0);
            let waiting = c.saved_generation.map(|n| generation.saturating_sub(n));
            let age_bonus = waiting.unwrap_or(0).min(64) as u32 * 10;
            let score = base + u32::from(boot_sector_missing) * 50 + age_bonus;
            Recommendation {
                disk: c.disk,
                attempt: c.attempt,
                image_sha256: c.image_sha256.into(),
                availability: c.availability,
                severity,
                missing_sectors: missing,
                total_sectors: c.total_sectors,
                boot_sector_missing,
                waiting_generations: waiting,
                score,
                reason: format!(
                    "{cost} ({missing}/{}); {}{}; {}. Score {score} estimates scheduling value/cost only, not recovery success. All partials remain eligible; no extra USB reread under the speed-first policy.",
                    c.total_sectors,
                    if c.availability == Availability::Ready { "removal confirmed" } else { "still held in USB; explicit transfer/removal required" },
                    if boot_sector_missing { "; boot sector missing (+50 priority, no inferred FAT layout)" } else { "" },
                    waiting.map_or("legacy queue age unknown (+0)".into(), |n| format!("{n} later ticket generations (+{age_bonus}, capped at 640)")),
                ),
            }
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|r| {
        (
            r.availability != Availability::Ready,
            std::cmp::Reverse(r.score),
            std::cmp::Reverse(r.waiting_generations.unwrap_or(0)),
            r.disk,
        )
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(disk: u32, bad: &[u64], saved_generation: Option<u64>) -> Candidate<'_> {
        Candidate {
            disk,
            attempt: 1,
            image_sha256: "fixture",
            total_sectors: 2880,
            bad,
            availability: Availability::Ready,
            saved_generation,
        }
    }

    #[test]
    fn severity_metadata_age_and_availability_have_explicit_bounded_effects() {
        let severe = (1..401).collect::<Vec<_>>();
        let localized = (1..10).collect::<Vec<_>>();
        let moderate = (1..100).collect::<Vec<_>>();
        let mut held = candidate(5, &[0], Some(1));
        held.availability = Availability::HeldInUsb;
        let ranked = rank(
            vec![
                candidate(1, &severe, Some(80)),
                candidate(2, &[1], Some(80)),
                candidate(3, &localized, Some(80)),
                candidate(4, &moderate, Some(80)),
                held,
            ],
            80,
        );
        assert_eq!(
            ranked.iter().map(|r| r.disk).collect::<Vec<_>>(),
            vec![2, 3, 4, 1, 5]
        );
        assert_eq!(ranked[3].severity, Severity::Severe);
        assert_eq!(ranked[4].score, 1190);
        assert!(
            ranked
                .iter()
                .all(|r| r.reason.contains("not recovery success"))
        );
        let aged = rank(
            vec![candidate(1, &severe, Some(1)), candidate(2, &[1], Some(80))],
            80,
        );
        assert_eq!(aged[0].disk, 1); // Severe work cannot be starved by newer easy disks.
        assert_eq!(aged[0].score, 740);
        assert_eq!(aged[0].waiting_generations, Some(79));
        let boot = rank(vec![candidate(1, &[1], None), candidate(2, &[0], None)], 80);
        assert_eq!(boot[0].disk, 2);
        assert!(boot[0].reason.contains("no inferred FAT layout"));
    }

    #[test]
    fn unknown_legacy_age_and_stable_ties_do_not_invent_waiting_time() {
        let ranked = rank(
            vec![candidate(3, &[1], None), candidate(1, &[1], None)],
            u64::MAX,
        );
        assert_eq!(
            ranked.iter().map(|r| r.disk).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(ranked[0].waiting_generations, None);
        assert_eq!(ranked[0].score, 500);
        assert!(ranked[0].reason.contains("age unknown"));
    }
}
