//! Step 4 for Windows installers (frames 09–10): the five Windows Setup
//! options, all off by default. They become the engine's `SetupOptions` and
//! an `autounattend.xml` on the stick; the account name is checked by the
//! engine's own rules. There is no password field: Windows Setup asks for
//! one, and OSflux never stores it.

use osflux_engine::{validate_account_name, SetupOptions};

/// What the person has ticked. The region is read from this PC when the
/// stick is written, not now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsOptions {
    pub bypass_requirements: bool,
    pub local_account: bool,
    pub account_name: String,
    pub skip_privacy_questions: bool,
    pub no_auto_bitlocker: bool,
    pub copy_region: bool,
}

/// The five options in the order the screen lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionId {
    BypassRequirements,
    LocalAccount,
    SkipPrivacyQuestions,
    NoAutoBitlocker,
    CopyRegion,
}

impl OptionId {
    pub const ALL: [OptionId; 5] = [
        Self::BypassRequirements,
        Self::LocalAccount,
        Self::SkipPrivacyQuestions,
        Self::NoAutoBitlocker,
        Self::CopyRegion,
    ];

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

impl WindowsOptions {
    pub fn set(&mut self, option: OptionId, on: bool) {
        match option {
            OptionId::BypassRequirements => self.bypass_requirements = on,
            OptionId::LocalAccount => self.local_account = on,
            OptionId::SkipPrivacyQuestions => self.skip_privacy_questions = on,
            OptionId::NoAutoBitlocker => self.no_auto_bitlocker = on,
            OptionId::CopyRegion => self.copy_region = on,
        }
    }

    pub fn is_on(&self, option: OptionId) -> bool {
        match option {
            OptionId::BypassRequirements => self.bypass_requirements,
            OptionId::LocalAccount => self.local_account,
            OptionId::SkipPrivacyQuestions => self.skip_privacy_questions,
            OptionId::NoAutoBitlocker => self.no_auto_bitlocker,
            OptionId::CopyRegion => self.copy_region,
        }
    }

    pub fn count(&self) -> usize {
        OptionId::ALL
            .iter()
            .filter(|option| self.is_on(**option))
            .count()
    }

    /// Why the account name can't be used, in the engine's words with a
    /// capital letter; `None` when it can, or when the option is off.
    pub fn account_error(&self) -> Option<String> {
        if !self.local_account {
            return None;
        }
        validate_account_name(self.account_name.trim())
            .err()
            .map(|reason| capitalize(&reason) + ".")
    }

    /// Next stays locked while the account name is not usable.
    pub fn can_continue(&self) -> bool {
        self.account_error().is_none()
    }

    /// The short labels for "This session" and the Confirm summary, such as
    /// `Skip hardware checks · Local account (alex)`, or `None`.
    pub fn summary(&self) -> String {
        let mut labels = Vec::new();
        if self.bypass_requirements {
            labels.push("Skip hardware checks".to_string());
        }
        if self.local_account {
            let name = self.account_name.trim();
            labels.push(if name.is_empty() {
                "Local account".to_string()
            } else {
                format!("Local account ({name})")
            });
        }
        if self.skip_privacy_questions {
            labels.push("Skip privacy questions".to_string());
        }
        if self.no_auto_bitlocker {
            labels.push("No automatic BitLocker".to_string());
        }
        if self.copy_region {
            labels.push("This PC's region and keyboard".to_string());
        }
        if labels.is_empty() {
            "None".to_string()
        } else {
            labels.join(" · ")
        }
    }

    /// The rail's subtitle once the step is done: "3 options on".
    pub fn rail_subtitle(&self) -> String {
        match self.count() {
            0 => "None".to_string(),
            1 => "1 option on".to_string(),
            count => format!("{count} options on"),
        }
    }

    /// The action bar's hint on this step.
    pub fn hint(&self) -> String {
        if let Some(error) = self.account_error() {
            return error;
        }
        match self.count() {
            0 => "All options are off. Windows Setup runs as normal.".to_string(),
            1 => "1 option is on. It applies to every PC that boots from this stick.".to_string(),
            count => {
                format!(
                    "{count} options are on. They apply to every PC that boots from this stick."
                )
            }
        }
    }

    /// The engine's options, except the region, which is read from this PC
    /// when the stick is written.
    pub fn setup_options(&self) -> SetupOptions {
        SetupOptions {
            bypass_requirements: self.bypass_requirements,
            local_account: self
                .local_account
                .then(|| self.account_name.trim().to_string()),
            skip_privacy_questions: self.skip_privacy_questions,
            disable_bitlocker_auto_encryption: self.no_auto_bitlocker,
            region: None,
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{OptionId, WindowsOptions};

    #[test]
    fn everything_is_off_by_default_as_in_frame_09() {
        let options = WindowsOptions::default();
        assert_eq!(options.count(), 0);
        assert_eq!(options.summary(), "None");
        assert_eq!(options.rail_subtitle(), "None");
        assert_eq!(
            options.hint(),
            "All options are off. Windows Setup runs as normal."
        );
        assert!(options.can_continue());
        assert!(options.setup_options().is_empty());
    }

    #[test]
    fn three_options_on_match_frame_10() {
        let mut options = WindowsOptions::default();
        options.set(OptionId::BypassRequirements, true);
        options.set(OptionId::LocalAccount, true);
        options.account_name = "alex".to_string();
        options.set(OptionId::SkipPrivacyQuestions, true);
        assert_eq!(options.count(), 3);
        assert_eq!(
            options.summary(),
            "Skip hardware checks · Local account (alex) · Skip privacy questions"
        );
        assert_eq!(options.rail_subtitle(), "3 options on");
        assert_eq!(
            options.hint(),
            "3 options are on. They apply to every PC that boots from this stick."
        );
        let setup = options.setup_options();
        assert!(setup.bypass_requirements);
        assert_eq!(setup.local_account.as_deref(), Some("alex"));
        assert!(setup.skip_privacy_questions);
        assert!(!setup.disable_bitlocker_auto_encryption);
    }

    #[test]
    fn the_account_name_follows_the_engine_rules() {
        let mut options = WindowsOptions::default();
        options.set(OptionId::LocalAccount, true);
        assert!(!options.can_continue(), "an empty name");
        assert!(options
            .account_error()
            .unwrap()
            .starts_with("The local account name is empty"));

        options.account_name = "Administrator".to_string();
        assert!(!options.can_continue(), "a name Windows keeps for itself");
        assert_eq!(options.hint(), options.account_error().unwrap());

        options.account_name = "  alex  ".to_string();
        assert!(options.can_continue());
        assert_eq!(
            options.setup_options().local_account.as_deref(),
            Some("alex")
        );

        options.set(OptionId::LocalAccount, false);
        options.account_name.clear();
        assert!(
            options.can_continue(),
            "the name only matters when the option is on"
        );
    }

    #[test]
    fn every_option_has_a_short_label() {
        let mut options = WindowsOptions::default();
        for option in OptionId::ALL {
            options.set(option, true);
        }
        options.account_name = "sam".to_string();
        assert_eq!(
            options.summary(),
            "Skip hardware checks · Local account (sam) · Skip privacy questions · No automatic BitLocker · This PC's region and keyboard"
        );
        assert_eq!(options.rail_subtitle(), "5 options on");
        assert_eq!(OptionId::from_index(4), Some(OptionId::CopyRegion));
        assert_eq!(OptionId::from_index(5), None);
    }
}
