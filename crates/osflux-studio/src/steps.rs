//! The step list in the rail: which steps there are, the state of each, and
//! "Step n of N". Windows ISOs get five steps (Windows options between
//! Safety check and Confirm); other images get four, and so does the rail
//! before an image is chosen.

use crate::format::middle_truncate;

/// The longest file name shown whole in a step subtitle or "This session":
/// 27 characters of Plex Mono fill the rail's text column.
pub const RAIL_NAME_CHARS: usize = 27;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// No image chosen yet.
    Undecided,
    /// A Windows installer ISO, with the Windows options step.
    Windows,
    /// A Linux ISO or a raw disk image.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    Image,
    Drive,
    Safety,
    WindowsOptions,
    Confirm,
}

impl StepId {
    pub fn title(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Drive => "Drive",
            Self::Safety => "Safety check",
            Self::WindowsOptions => "Windows options",
            Self::Confirm => "Confirm",
        }
    }

    /// The subtitle of a step that has not been reached.
    pub fn waiting_subtitle(self) -> &'static str {
        match self {
            Self::Image => "Not chosen yet",
            Self::Drive => "Pick a USB stick or SD card",
            Self::Safety => "The engine checks everything",
            Self::WindowsOptions => "Optional Setup changes",
            Self::Confirm => "Confirm the drive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Current,
    Done,
    Blocked,
}

/// What a step's subtitle says, when it is not the waiting text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subtitle {
    /// Plain text, such as "SanDisk Ultra 64GB" or "Verdict: Ready".
    Text(String),
    /// A file name: Plex Mono, shortened in the middle when long.
    FileName(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepView {
    pub id: StepId,
    pub title: &'static str,
    pub subtitle: String,
    pub subtitle_mono: bool,
    /// The whole subtitle, for the hover tooltip; empty when not shortened.
    pub subtitle_full: String,
    pub state: StepState,
    /// Done steps can be clicked to go back to them.
    pub clickable: bool,
}

pub fn steps_for(flow: Flow) -> &'static [StepId] {
    match flow {
        Flow::Windows => &[
            StepId::Image,
            StepId::Drive,
            StepId::Safety,
            StepId::WindowsOptions,
            StepId::Confirm,
        ],
        Flow::Undecided | Flow::Other => &[
            StepId::Image,
            StepId::Drive,
            StepId::Safety,
            StepId::Confirm,
        ],
    }
}

/// "Step 1" before an image is chosen, then "Step n of N".
pub fn step_label(flow: Flow, current: StepId) -> String {
    let steps = steps_for(flow);
    let number = steps
        .iter()
        .position(|step| *step == current)
        .map(|index| index + 1)
        .unwrap_or(1);
    match flow {
        Flow::Undecided => format!("Step {number}"),
        Flow::Windows | Flow::Other => format!("Step {number} of {}", steps.len()),
    }
}

/// Builds the rail. Steps before `current` are done, later ones pending.
/// `blocked` marks a step red (the current step included), and `subtitles`
/// replaces the waiting text of the steps it names.
pub fn build_rail(
    flow: Flow,
    current: StepId,
    blocked: Option<StepId>,
    subtitles: &[(StepId, Subtitle)],
) -> Vec<StepView> {
    let steps = steps_for(flow);
    let current_index = steps.iter().position(|step| *step == current).unwrap_or(0);

    steps
        .iter()
        .enumerate()
        .map(|(index, &id)| {
            let state = if blocked == Some(id) {
                StepState::Blocked
            } else if index < current_index {
                StepState::Done
            } else if index == current_index {
                StepState::Current
            } else {
                StepState::Pending
            };
            let subtitle = subtitles
                .iter()
                .find(|(step, _)| *step == id)
                .map(|(_, subtitle)| subtitle.clone())
                .unwrap_or_else(|| Subtitle::Text(id.waiting_subtitle().to_string()));
            let (subtitle, subtitle_mono, subtitle_full) = match subtitle {
                Subtitle::Text(text) => (text, false, String::new()),
                Subtitle::FileName(name) => {
                    let shortened = middle_truncate(&name, RAIL_NAME_CHARS);
                    let full = if shortened == name {
                        String::new()
                    } else {
                        name
                    };
                    (shortened, true, full)
                }
            };
            StepView {
                id,
                title: id.title(),
                subtitle,
                subtitle_mono,
                subtitle_full,
                state,
                clickable: state == StepState::Done,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{build_rail, step_label, steps_for, Flow, StepId, StepState, Subtitle};

    #[test]
    fn windows_isos_get_five_steps_and_others_four() {
        assert_eq!(steps_for(Flow::Windows).len(), 5);
        assert_eq!(steps_for(Flow::Windows)[3], StepId::WindowsOptions);
        assert_eq!(steps_for(Flow::Other).len(), 4);
        assert_eq!(steps_for(Flow::Undecided).len(), 4);
        assert!(!steps_for(Flow::Other).contains(&StepId::WindowsOptions));
    }

    #[test]
    fn step_labels_match_the_frames() {
        assert_eq!(step_label(Flow::Undecided, StepId::Image), "Step 1");
        assert_eq!(step_label(Flow::Windows, StepId::Image), "Step 1 of 5");
        assert_eq!(
            step_label(Flow::Windows, StepId::WindowsOptions),
            "Step 4 of 5"
        );
        assert_eq!(step_label(Flow::Windows, StepId::Confirm), "Step 5 of 5");
        assert_eq!(step_label(Flow::Other, StepId::Confirm), "Step 4 of 4");
    }

    #[test]
    fn the_empty_rail_matches_frame_01() {
        let rail = build_rail(Flow::Undecided, StepId::Image, None, &[]);
        let states: Vec<_> = rail.iter().map(|step| step.state).collect();
        assert_eq!(
            states,
            [
                StepState::Current,
                StepState::Pending,
                StepState::Pending,
                StepState::Pending
            ]
        );
        let subtitles: Vec<_> = rail.iter().map(|step| step.subtitle.as_str()).collect();
        assert_eq!(
            subtitles,
            [
                "Not chosen yet",
                "Pick a USB stick or SD card",
                "The engine checks everything",
                "Confirm the drive"
            ]
        );
        assert!(rail.iter().all(|step| !step.clickable));
    }

    #[test]
    fn earlier_steps_are_done_and_clickable() {
        let rail = build_rail(
            Flow::Windows,
            StepId::Safety,
            None,
            &[
                (
                    StepId::Image,
                    Subtitle::FileName("Windows11_Client_x64_en-gb_26300_9457.iso".into()),
                ),
                (StepId::Drive, Subtitle::Text("SanDisk Ultra 64GB".into())),
            ],
        );
        assert_eq!(rail[0].state, StepState::Done);
        assert!(rail[0].clickable);
        assert_eq!(rail[0].subtitle, "Windows11_Client_…_9457.iso");
        assert!(rail[0].subtitle_mono);
        assert_eq!(
            rail[0].subtitle_full,
            "Windows11_Client_x64_en-gb_26300_9457.iso"
        );
        assert_eq!(rail[1].subtitle, "SanDisk Ultra 64GB");
        assert!(!rail[1].subtitle_mono);
        assert_eq!(rail[2].state, StepState::Current);
        assert!(!rail[2].clickable);
        assert_eq!(rail[3].state, StepState::Pending);
        assert_eq!(rail[3].subtitle, "Optional Setup changes");
    }

    #[test]
    fn a_blocked_step_is_red_and_not_clickable() {
        // Frame 08: the safety check itself is blocked.
        let rail = build_rail(
            Flow::Windows,
            StepId::Safety,
            Some(StepId::Safety),
            &[(
                StepId::Safety,
                Subtitle::Text("Blocked: image too large".into()),
            )],
        );
        assert_eq!(rail[2].state, StepState::Blocked);
        assert!(!rail[2].clickable);

        // Frame 22: the drive was unplugged while Confirm is current.
        let rail = build_rail(Flow::Windows, StepId::Confirm, Some(StepId::Drive), &[]);
        assert_eq!(rail[1].state, StepState::Blocked);
        assert!(!rail[1].clickable);
        assert_eq!(rail[4].state, StepState::Current);
    }

    #[test]
    fn short_file_names_have_no_tooltip() {
        let rail = build_rail(
            Flow::Other,
            StepId::Image,
            None,
            &[(
                StepId::Image,
                Subtitle::FileName("recovery-tools-v3.iso".into()),
            )],
        );
        assert_eq!(rail[0].subtitle, "recovery-tools-v3.iso");
        assert!(rail[0].subtitle_full.is_empty());
    }
}
