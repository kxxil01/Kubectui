//! Centralized icon registry with configurable display modes.
//!
//! Three modes: Nerd Font (default), Emoji, Plain text. Follows the same
//! global-static pattern as the theme system (`active_icon_mode` / `set_icon_mode`).

use std::sync::atomic::{AtomicU8, Ordering};

use crate::app::views::AppView;

// ── Icon mode ──────────────────────────────────────────────────────

/// Display mode for icons across the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconMode {
    /// Nerd Font glyphs (requires patched font). Default.
    Nerd = 0,
    /// Standard Unicode emoji (works in most terminals).
    Emoji = 1,
    /// No icons, plain text labels only.
    Plain = 2,
}

const ICON_MODE_COUNT: u8 = 3;
static ACTIVE_ICON_MODE: AtomicU8 = AtomicU8::new(0);

/// Sets the active icon mode.
pub fn set_icon_mode(mode: IconMode) {
    ACTIVE_ICON_MODE.store(mode as u8, Ordering::Relaxed);
}

/// Returns the current icon mode.
pub fn active_icon_mode() -> IconMode {
    match ACTIVE_ICON_MODE.load(Ordering::Relaxed) {
        1 => IconMode::Emoji,
        2 => IconMode::Plain,
        _ => IconMode::Nerd,
    }
}

/// Cycles to the next icon mode and returns it.
pub fn cycle_icon_mode() -> IconMode {
    let next = (ACTIVE_ICON_MODE.load(Ordering::Relaxed) + 1) % ICON_MODE_COUNT;
    ACTIVE_ICON_MODE.store(next, Ordering::Relaxed);
    active_icon_mode()
}

/// Parses an icon mode from a config string.
pub fn parse_icon_mode(s: &str) -> IconMode {
    match s {
        "emoji" => IconMode::Emoji,
        "plain" => IconMode::Plain,
        _ => IconMode::Nerd,
    }
}

/// Returns the config string for an icon mode.
pub fn icon_mode_name(mode: IconMode) -> &'static str {
    match mode {
        IconMode::Nerd => "nerd",
        IconMode::Emoji => "emoji",
        IconMode::Plain => "plain",
    }
}

// ── Icon entry ─────────────────────────────────────────────────────

/// A single icon with variants for each display mode.
#[derive(Debug, Clone, Copy)]
pub struct Icon {
    pub nerd: &'static str,
    pub emoji: &'static str,
    pub plain: &'static str,
}

impl Icon {
    /// Returns the icon string for the given mode.
    pub const fn for_mode(&self, mode: IconMode) -> &'static str {
        match mode {
            IconMode::Nerd => self.nerd,
            IconMode::Emoji => self.emoji,
            IconMode::Plain => self.plain,
        }
    }

    /// Returns the icon string for the currently active mode.
    pub fn active(&self) -> &'static str {
        self.for_mode(active_icon_mode())
    }
}

// ── View icons ─────────────────────────────────────────────────────

/// Returns the icon for a given view.
pub fn view_icon(view: AppView) -> Icon {
    match view {
        // Overview
        AppView::Dashboard => Icon {
            nerd: "󰋗 ",
            emoji: "📊 ",
            plain: "",
        },
        AppView::Bookmarks => Icon {
            nerd: "󰃀 ",
            emoji: "⭐ ",
            plain: "",
        },
        AppView::HealthReport => Icon {
            nerd: "󰓶 ",
            emoji: "🩺 ",
            plain: "",
        },
        AppView::Issues => Icon {
            nerd: "󰀬 ",
            emoji: "⚠ ",
            plain: "",
        },

        // Cluster
        AppView::Nodes => Icon {
            nerd: "󰒋 ",
            emoji: "🖥 ",
            plain: "",
        },

        // Workloads
        AppView::Pods => Icon {
            nerd: "󰠳 ",
            emoji: "🐳 ",
            plain: "",
        },
        AppView::Deployments => Icon {
            nerd: "󰜟 ",
            emoji: "🚀 ",
            plain: "",
        },
        AppView::StatefulSets => Icon {
            nerd: "󰆼 ",
            emoji: "🗄 ",
            plain: "",
        },
        AppView::DaemonSets => Icon {
            nerd: "👾 ",
            emoji: "👾 ",
            plain: "",
        },
        AppView::ReplicaSets => Icon {
            nerd: "󰆧 ",
            emoji: "🔁 ",
            plain: "",
        },
        AppView::ReplicationControllers => Icon {
            nerd: "󰆧 ",
            emoji: "🔄 ",
            plain: "",
        },
        AppView::Jobs => Icon {
            nerd: "󰃰 ",
            emoji: "⚙ ",
            plain: "",
        },
        AppView::CronJobs => Icon {
            nerd: "󰔠 ",
            emoji: "🕐 ",
            plain: "",
        },

        // Network
        AppView::Services => Icon {
            nerd: "󰛳 ",
            emoji: "🔌 ",
            plain: "",
        },
        AppView::Endpoints => Icon {
            nerd: "󰟐 ",
            emoji: "📍 ",
            plain: "",
        },
        AppView::Ingresses => Icon {
            nerd: "󰱓 ",
            emoji: "🌐 ",
            plain: "",
        },
        AppView::IngressClasses => Icon {
            nerd: "󰱓 ",
            emoji: "🏷 ",
            plain: "",
        },
        AppView::NetworkPolicies => Icon {
            nerd: "󰒃 ",
            emoji: "🛡 ",
            plain: "",
        },
        AppView::PortForwarding => Icon {
            nerd: "󰛳 ",
            emoji: "🔀 ",
            plain: "",
        },

        // Config & Governance
        AppView::ConfigMaps => Icon {
            nerd: "󰒓 ",
            emoji: "📄 ",
            plain: "",
        },
        AppView::Secrets => Icon {
            nerd: "󰌋 ",
            emoji: "🔐 ",
            plain: "",
        },
        AppView::ResourceQuotas => Icon {
            nerd: "󰏗 ",
            emoji: "📊 ",
            plain: "",
        },
        AppView::LimitRanges => Icon {
            nerd: "󰳗 ",
            emoji: "⚖ ",
            plain: "",
        },
        AppView::HPAs => Icon {
            nerd: "󰦕 ",
            emoji: "📈 ",
            plain: "",
        },
        AppView::PodDisruptionBudgets => Icon {
            nerd: "󰦕 ",
            emoji: "🛡 ",
            plain: "",
        },
        AppView::PriorityClasses => Icon {
            nerd: "󰔠 ",
            emoji: "⭐ ",
            plain: "",
        },
        AppView::Namespaces => Icon {
            nerd: "󰏗 ",
            emoji: "📁 ",
            plain: "",
        },
        AppView::Events => Icon {
            nerd: "󰃰 ",
            emoji: "📋 ",
            plain: "",
        },

        // Storage
        AppView::PersistentVolumeClaims => Icon {
            nerd: "󰋊 ",
            emoji: "💾 ",
            plain: "",
        },
        AppView::PersistentVolumes => Icon {
            nerd: "󰋊 ",
            emoji: "🗃 ",
            plain: "",
        },
        AppView::StorageClasses => Icon {
            nerd: "󰋊 ",
            emoji: "🏗 ",
            plain: "",
        },

        // RBAC
        AppView::ServiceAccounts => Icon {
            nerd: "󰀄 ",
            emoji: "🔑 ",
            plain: "",
        },
        AppView::Roles => Icon {
            nerd: "󰒃 ",
            emoji: "🛡 ",
            plain: "",
        },
        AppView::ClusterRoles => Icon {
            nerd: "󰒃 ",
            emoji: "🏰 ",
            plain: "",
        },
        AppView::RoleBindings => Icon {
            nerd: "󰌋 ",
            emoji: "🔗 ",
            plain: "",
        },
        AppView::ClusterRoleBindings => Icon {
            nerd: "󰌋 ",
            emoji: "⛓ ",
            plain: "",
        },

        // Helm
        AppView::HelmCharts => Icon {
            nerd: "󰱥 ",
            emoji: "📦 ",
            plain: "",
        },
        AppView::HelmReleases => Icon {
            nerd: "󰱥 ",
            emoji: "⎈ ",
            plain: "",
        },

        // Flux
        AppView::FluxCDAll
        | AppView::FluxCDAlertProviders
        | AppView::FluxCDAlerts
        | AppView::FluxCDArtifacts
        | AppView::FluxCDHelmReleases
        | AppView::FluxCDHelmRepositories
        | AppView::FluxCDImages
        | AppView::FluxCDKustomizations
        | AppView::FluxCDReceivers
        | AppView::FluxCDSources => Icon {
            nerd: "󰠳 ",
            emoji: "🌀 ",
            plain: "",
        },

        // Extensions
        AppView::Extensions => Icon {
            nerd: "󰏗 ",
            emoji: "🧩 ",
            plain: "",
        },
    }
}

// ── Nav group icons ────────────────────────────────────────────────

/// Returns the icon for a sidebar navigation group.
pub fn group_icon(group: &str) -> Icon {
    match group {
        "Overview" => Icon {
            nerd: "󰋗 ",
            emoji: "📊 ",
            plain: "",
        },
        "Workloads" => Icon {
            nerd: "󰆧 ",
            emoji: "🚀 ",
            plain: "",
        },
        "Network" => Icon {
            nerd: "󰛳 ",
            emoji: "🌐 ",
            plain: "",
        },
        "Config" => Icon {
            nerd: "󰒓 ",
            emoji: "📄 ",
            plain: "",
        },
        "Storage" => Icon {
            nerd: "󰋊 ",
            emoji: "💾 ",
            plain: "",
        },
        "Helm" => Icon {
            nerd: "󰱥 ",
            emoji: "⎈ ",
            plain: "",
        },
        "FluxCD" => Icon {
            nerd: "󰠳 ",
            emoji: "🌀 ",
            plain: "",
        },
        "Access Control" => Icon {
            nerd: "󰒃 ",
            emoji: "🔐 ",
            plain: "",
        },
        "Custom Resources" => Icon {
            nerd: "󰏗 ",
            emoji: "🧩 ",
            plain: "",
        },
        _ => Icon {
            nerd: "",
            emoji: "",
            plain: "",
        },
    }
}

// ── Workbench tab icons ────────────────────────────────────────────

/// Returns the icon for a workbench tab kind.
pub fn tab_icon(kind: &str) -> Icon {
    match kind {
        "History" => Icon {
            nerd: "󰋚 ",
            emoji: "📜 ",
            plain: "",
        },
        "YAML" => Icon {
            nerd: "󰗀 ",
            emoji: "📝 ",
            plain: "",
        },
        "Decoded" => Icon {
            nerd: "󰌋 ",
            emoji: "🔓 ",
            plain: "",
        },
        "Timeline" => Icon {
            nerd: "󰃰 ",
            emoji: "📅 ",
            plain: "",
        },
        "Logs" => Icon {
            nerd: "󰆍 ",
            emoji: "📃 ",
            plain: "",
        },
        "Workload Logs" => Icon {
            nerd: "󰆍 ",
            emoji: "📃 ",
            plain: "",
        },
        "Exec" => Icon {
            nerd: "󰆍 ",
            emoji: "💻 ",
            plain: "",
        },
        "Port-Forward" => Icon {
            nerd: "󰛳 ",
            emoji: "🔀 ",
            plain: "",
        },
        "Relations" => Icon {
            nerd: "󰙅 ",
            emoji: "🔗 ",
            plain: "",
        },
        "NetPol" => Icon {
            nerd: "󰒃 ",
            emoji: "🛡 ",
            plain: "",
        },
        "Reach" => Icon {
            nerd: "󰛳 ",
            emoji: "🧭 ",
            plain: "",
        },
        _ => Icon {
            nerd: "",
            emoji: "",
            plain: "",
        },
    }
}

// ── Status icons ───────────────────────────────────────────────────

/// Status indicator icons.
pub struct StatusIcons;

impl StatusIcons {
    pub fn error() -> Icon {
        Icon {
            nerd: "󰅙 ",
            emoji: "✗ ",
            plain: "[!] ",
        }
    }
    pub fn warning() -> Icon {
        Icon {
            nerd: "󰀦 ",
            emoji: "⚠ ",
            plain: "[?] ",
        }
    }
    pub fn info() -> Icon {
        Icon {
            nerd: "󰋼 ",
            emoji: "ℹ ",
            plain: "[i] ",
        }
    }
    pub fn bookmark() -> Icon {
        Icon {
            nerd: "󰃀 ",
            emoji: "★ ",
            plain: "[*] ",
        }
    }
    pub fn bookmark_missing() -> Icon {
        Icon {
            nerd: "󰅖 ",
            emoji: "✗ ",
            plain: "[x] ",
        }
    }
}

// ── UI chrome icons ────────────────────────────────────────────────

/// Returns an icon for UI chrome elements (headers, pickers, dashboard sections).
pub fn chrome_icon(name: &str) -> Icon {
    match name {
        "cluster" => Icon {
            nerd: "󰠳 ",
            emoji: "⎈ ",
            plain: "",
        },
        "cloud" => Icon {
            nerd: "󰅟 ",
            emoji: "⛅ ",
            plain: "",
        },
        "resources" => Icon {
            nerd: "󰋗 ",
            emoji: "📊 ",
            plain: "",
        },
        "governance" => Icon {
            nerd: "󰳗 ",
            emoji: "⚖ ",
            plain: "",
        },
        _ => Icon {
            nerd: "",
            emoji: "",
            plain: "",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_nerd() {
        // Reset to default
        ACTIVE_ICON_MODE.store(0, Ordering::Relaxed);
        assert_eq!(active_icon_mode(), IconMode::Nerd);
    }

    #[test]
    fn cycle_mode_wraps_around() {
        ACTIVE_ICON_MODE.store(0, Ordering::Relaxed);
        assert_eq!(cycle_icon_mode(), IconMode::Emoji);
        assert_eq!(cycle_icon_mode(), IconMode::Plain);
        assert_eq!(cycle_icon_mode(), IconMode::Nerd);
    }

    #[test]
    fn parse_round_trips() {
        for mode in [IconMode::Nerd, IconMode::Emoji, IconMode::Plain] {
            assert_eq!(parse_icon_mode(icon_mode_name(mode)), mode);
        }
    }

    #[test]
    fn view_icon_returns_correct_mode() {
        let icon = view_icon(AppView::Pods);
        assert_eq!(icon.for_mode(IconMode::Nerd), "󰠳 ");
        assert_eq!(icon.for_mode(IconMode::Emoji), "🐳 ");
        assert_eq!(icon.for_mode(IconMode::Plain), "");
    }

    #[test]
    fn plain_mode_returns_empty() {
        let icon = view_icon(AppView::Deployments);
        assert!(icon.for_mode(IconMode::Plain).is_empty());
    }

    #[test]
    fn group_icon_unknown_returns_empty() {
        let icon = group_icon("NonExistent");
        assert!(icon.nerd.is_empty());
    }

    #[test]
    fn tab_icon_known_kinds() {
        let icon = tab_icon("YAML");
        assert!(!icon.nerd.is_empty());
        assert!(!icon.emoji.is_empty());
    }

    #[test]
    fn status_icons_non_empty() {
        assert!(!StatusIcons::error().nerd.is_empty());
        assert!(!StatusIcons::warning().emoji.is_empty());
        assert!(!StatusIcons::info().plain.is_empty());
    }
}
