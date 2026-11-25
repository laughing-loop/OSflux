//! Windows Setup options, written as an answer file (`autounattend.xml`) at
//! the root of the stick. Windows Setup reads it from there by itself.
//!
//! Every option is off unless asked for. The answer file never holds a
//! password: the local account starts with a blank one, and Windows asks for
//! a new one at the next sign-in.

use std::fmt::Write as _;

use osflux_core::region::is_input_method_tip;
pub use osflux_core::region::RegionSettings;

/// The name Windows Setup looks for at the root of removable drives.
pub const ANSWER_FILE_NAME: &str = "autounattend.xml";

/// Names Windows keeps for itself, compared without regard to case.
const RESERVED_ACCOUNT_NAMES: &[&str] = &[
    "administrator",
    "administrators",
    "defaultaccount",
    "defaultuser0",
    "guest",
    "guests",
    "none",
    "system",
    "users",
    "wdagutilityaccount",
];
const MAX_ACCOUNT_NAME_CHARS: usize = 20;

/// What Windows Setup should do differently from a stock install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupOptions {
    /// Skip Setup's TPM 2.0, Secure Boot and 4 GB memory checks.
    pub bypass_requirements: bool,
    /// Create this local account instead of asking for a Microsoft account.
    pub local_account: Option<String>,
    /// Answer the privacy questions with the choices that send the least
    /// data to Microsoft, so they are not asked.
    pub skip_privacy_questions: bool,
    /// Stop Windows from turning on BitLocker device encryption by itself.
    pub disable_bitlocker_auto_encryption: bool,
    /// Use these region and keyboard settings instead of asking for them.
    pub region: Option<RegionSettings>,
}

/// The processor architecture of the Windows on the ISO, which every
/// component in the answer file must name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Amd64,
    Arm64,
    X86,
}

impl Architecture {
    /// From the name of the ISO's UEFI loader, such as `bootx64.efi`.
    pub fn from_uefi_loader(name: &str) -> Option<Self> {
        let name = name.rsplit('/').next()?.to_ascii_lowercase();
        match name.as_str() {
            "bootx64.efi" => Some(Self::Amd64),
            "bootaa64.efi" => Some(Self::Arm64),
            "bootia32.efi" => Some(Self::X86),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::X86 => "x86",
        }
    }
}

impl SetupOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// One line per option, for the write plan.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.bypass_requirements {
            lines.push("skip the TPM 2.0, Secure Boot and 4 GB memory checks".to_string());
        }
        if let Some(name) = &self.local_account {
            lines.push(format!(
                "local account \"{name}\" instead of a Microsoft account, with a blank password: it signs in once by itself to finish setup, then Windows asks for a new password at the next sign-in; passwords are set never to expire"
            ));
        }
        if self.skip_privacy_questions {
            lines.push(
                "skip the privacy questions, choosing the least data sent to Microsoft".to_string(),
            );
        }
        if self.disable_bitlocker_auto_encryption {
            lines.push("no automatic BitLocker device encryption".to_string());
        }
        if let Some(region) = &self.region {
            let keyboards: Vec<&str> = region.input_locale.split(';').collect();
            let keyboards = match keyboards.as_slice() {
                [only] => format!("keyboard {only}"),
                [first, rest @ ..] => format!("keyboards {first} (default), {}", rest.join(", ")),
                [] => String::new(),
            };
            lines.push(format!(
                "region and keyboard copied from this PC: formats {}, system locale {}, {keyboards}",
                region.user_locale, region.system_locale
            ));
        }
        lines
    }

    /// A short list for the audit log, such as
    /// `bypass-requirements, local-account=alex`.
    pub fn audit_summary(&self) -> String {
        let mut parts = Vec::new();
        if self.bypass_requirements {
            parts.push("bypass-requirements".to_string());
        }
        if let Some(name) = &self.local_account {
            parts.push(format!("local-account={name}"));
        }
        if self.skip_privacy_questions {
            parts.push("skip-privacy-questions".to_string());
        }
        if self.disable_bitlocker_auto_encryption {
            parts.push("no-bitlocker-auto-encryption".to_string());
        }
        if let Some(region) = &self.region {
            parts.push(format!(
                "region={}/{}/{}",
                region.user_locale, region.system_locale, region.input_locale
            ));
        }
        parts.join(", ")
    }

    /// Checks every value that ends up in the answer file.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(name) = &self.local_account {
            validate_account_name(name)?;
        }
        if let Some(region) = &self.region {
            validate_region(region)?;
        }
        Ok(())
    }
}

/// Windows account names: at most 20 characters, no characters Windows or a
/// command line treats specially, and not a name Windows keeps for itself.
/// Letters of any script, digits, spaces, periods, hyphens and underscores
/// are allowed.
pub fn validate_account_name(name: &str) -> Result<(), String> {
    let length = name.chars().count();
    if length == 0 {
        return Err("the local account name is empty".to_string());
    }
    if length > MAX_ACCOUNT_NAME_CHARS {
        return Err(format!(
            "the local account name \"{name}\" is {length} characters; Windows allows at most {MAX_ACCOUNT_NAME_CHARS}"
        ));
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_')))
    {
        return Err(format!(
            "the local account name \"{name}\" contains {bad:?}; use letters, digits, spaces, periods, hyphens or underscores"
        ));
    }
    if name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
        return Err(format!(
            "the local account name \"{name}\" cannot start with a space or end with a space or period"
        ));
    }
    if RESERVED_ACCOUNT_NAMES.contains(&name.to_lowercase().as_str()) {
        return Err(format!(
            "\"{name}\" is a name Windows keeps for itself; choose another local account name"
        ));
    }
    Ok(())
}

fn validate_region(region: &RegionSettings) -> Result<(), String> {
    let locale_ok = |value: &str| {
        !value.is_empty()
            && value.len() <= 85
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    for (what, value) in [
        ("system locale", &region.system_locale),
        ("user locale", &region.user_locale),
    ] {
        if !locale_ok(value) {
            return Err(format!(
                "the {what} {value:?} is not a locale name such as en-GB"
            ));
        }
    }
    let keyboards: Vec<&str> = region.input_locale.split(';').collect();
    if keyboards.is_empty() || !keyboards.iter().all(|tip| is_input_method_tip(tip)) {
        return Err(format!(
            "the keyboard list {:?} is not in the form 0809:00020409;0809:00000809",
            region.input_locale
        ));
    }
    Ok(())
}

/// The answer file for `options`, or `None` when no option is on. The
/// output depends only on the options and the architecture, so the planned
/// file and the written one are the same bytes.
pub fn answer_file(options: &SetupOptions, architecture: Architecture) -> Option<String> {
    if options.is_empty() {
        return None;
    }
    let arch = architecture.as_str();
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n");
    xml.push_str(
        "<!-- Written by OSflux. Windows Setup reads this file from the root of the drive. -->\r\n",
    );
    for line in options.describe() {
        let _ = write!(xml, "<!-- {} -->\r\n", comment_text(&line));
    }
    xml.push_str("<unattend xmlns=\"urn:schemas-microsoft-com:unattend\" xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\">\r\n");

    if options.bypass_requirements {
        // Setup checks these values before it checks the PC. They live in
        // the Windows PE registry only, so the installed Windows never
        // sees them.
        let commands: Vec<String> = ["BypassTPMCheck", "BypassSecureBootCheck", "BypassRAMCheck"]
            .iter()
            .map(|value| {
                format!(r"reg add HKLM\SYSTEM\Setup\LabConfig /v {value} /t REG_DWORD /d 1 /f")
            })
            .collect();
        open_pass(&mut xml, "windowsPE");
        open_component(&mut xml, "Microsoft-Windows-Setup", arch);
        run_synchronous(&mut xml, &commands);
        close_component(&mut xml);
        close_pass(&mut xml);
    }

    let mut specialize = Vec::new();
    if options.local_account.is_some() {
        // Lets the first-start screens go on without a network connection,
        // which a Microsoft account would need.
        specialize.push(
            r"reg add HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE /v BypassNRO /t REG_DWORD /d 1 /f"
                .to_string(),
        );
    }
    if options.disable_bitlocker_auto_encryption {
        specialize.push(
            r"reg add HKLM\SYSTEM\CurrentControlSet\Control\BitLocker /v PreventDeviceEncryption /t REG_DWORD /d 1 /f"
                .to_string(),
        );
    }
    if !specialize.is_empty() {
        open_pass(&mut xml, "specialize");
        open_component(&mut xml, "Microsoft-Windows-Deployment", arch);
        run_synchronous(&mut xml, &specialize);
        close_component(&mut xml);
        close_pass(&mut xml);
    }

    let shell_setup = options.local_account.is_some() || options.skip_privacy_questions;
    if options.region.is_some() || shell_setup {
        open_pass(&mut xml, "oobeSystem");
        if let Some(region) = &options.region {
            open_component(&mut xml, "Microsoft-Windows-International-Core", arch);
            element(&mut xml, 6, "InputLocale", &region.input_locale);
            element(&mut xml, 6, "SystemLocale", &region.system_locale);
            element(&mut xml, 6, "UserLocale", &region.user_locale);
            close_component(&mut xml);
        }
        if shell_setup {
            open_component(&mut xml, "Microsoft-Windows-Shell-Setup", arch);
            if let Some(name) = &options.local_account {
                xml.push_str("      <AutoLogon>\r\n");
                element(&mut xml, 8, "Username", name);
                element(&mut xml, 8, "Enabled", "true");
                element(&mut xml, 8, "LogonCount", "1");
                blank_password(&mut xml, 8);
                xml.push_str("      </AutoLogon>\r\n");
                xml.push_str("      <FirstLogonCommands>\r\n");
                let commands = [
                    (
                        format!("net user \"{name}\" /logonpasswordchg:yes"),
                        "Ask for a new password at the next sign-in",
                    ),
                    (
                        "net accounts /maxpwage:unlimited".to_string(),
                        "Keep the new password from expiring",
                    ),
                ];
                for (order, (command, description)) in commands.iter().enumerate() {
                    xml.push_str("        <SynchronousCommand wcm:action=\"add\">\r\n");
                    element(&mut xml, 10, "Order", &(order + 1).to_string());
                    element(&mut xml, 10, "CommandLine", command);
                    element(&mut xml, 10, "Description", description);
                    xml.push_str("        </SynchronousCommand>\r\n");
                }
                xml.push_str("      </FirstLogonCommands>\r\n");
            }
            xml.push_str("      <OOBE>\r\n");
            if options.local_account.is_some() {
                element(&mut xml, 8, "HideOnlineAccountScreens", "true");
            }
            if options.skip_privacy_questions {
                // 3: turn off the recommended settings that send data.
                element(&mut xml, 8, "ProtectYourPC", "3");
            }
            xml.push_str("      </OOBE>\r\n");
            if let Some(name) = &options.local_account {
                xml.push_str("      <UserAccounts>\r\n");
                xml.push_str("        <LocalAccounts>\r\n");
                xml.push_str("          <LocalAccount wcm:action=\"add\">\r\n");
                element(&mut xml, 12, "Name", name);
                element(&mut xml, 12, "DisplayName", name);
                element(&mut xml, 12, "Group", "Administrators");
                blank_password(&mut xml, 12);
                xml.push_str("          </LocalAccount>\r\n");
                xml.push_str("        </LocalAccounts>\r\n");
                xml.push_str("      </UserAccounts>\r\n");
            }
            close_component(&mut xml);
        }
        close_pass(&mut xml);
    }

    xml.push_str("</unattend>\r\n");
    Some(xml)
}

fn open_pass(xml: &mut String, pass: &str) {
    let _ = write!(xml, "  <settings pass=\"{pass}\">\r\n");
}

fn close_pass(xml: &mut String) {
    xml.push_str("  </settings>\r\n");
}

fn open_component(xml: &mut String, name: &str, arch: &str) {
    let _ = write!(
        xml,
        "    <component name=\"{name}\" processorArchitecture=\"{arch}\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\r\n"
    );
}

fn close_component(xml: &mut String) {
    xml.push_str("    </component>\r\n");
}

fn run_synchronous(xml: &mut String, commands: &[String]) {
    xml.push_str("      <RunSynchronous>\r\n");
    for (order, command) in commands.iter().enumerate() {
        xml.push_str("        <RunSynchronousCommand wcm:action=\"add\">\r\n");
        element(xml, 10, "Order", &(order + 1).to_string());
        element(xml, 10, "Path", command);
        xml.push_str("        </RunSynchronousCommand>\r\n");
    }
    xml.push_str("      </RunSynchronous>\r\n");
}

/// An empty password: Windows reads an empty plain-text value as no
/// password at all.
fn blank_password(xml: &mut String, indent: usize) {
    let pad = " ".repeat(indent);
    let _ = write!(xml, "{pad}<Password>\r\n");
    element(xml, indent + 2, "Value", "");
    element(xml, indent + 2, "PlainText", "true");
    let _ = write!(xml, "{pad}</Password>\r\n");
}

fn element(xml: &mut String, indent: usize, name: &str, value: &str) {
    let _ = write!(
        xml,
        "{}<{name}>{}</{name}>\r\n",
        " ".repeat(indent),
        escape(value)
    );
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// XML comments cannot hold `--`; everything else is literal in them.
fn comment_text(text: &str) -> String {
    text.replace("--", "- -")
}

#[cfg(test)]
mod tests {
    use super::{answer_file, validate_account_name, Architecture, RegionSettings, SetupOptions};

    fn all_options() -> SetupOptions {
        SetupOptions {
            bypass_requirements: true,
            local_account: Some("alex".to_string()),
            skip_privacy_questions: true,
            disable_bitlocker_auto_encryption: true,
            region: Some(RegionSettings {
                input_locale: "0809:00000809".to_string(),
                system_locale: "en-GB".to_string(),
                user_locale: "en-GB".to_string(),
            }),
        }
    }

    fn only(change: impl FnOnce(&mut SetupOptions)) -> String {
        let mut options = SetupOptions::default();
        change(&mut options);
        answer_file(&options, Architecture::Amd64).unwrap()
    }

    #[test]
    fn writes_no_answer_file_without_options() {
        assert!(answer_file(&SetupOptions::default(), Architecture::Amd64).is_none());
        assert!(SetupOptions::default().describe().is_empty());
    }

    #[test]
    fn skips_the_hardware_checks_in_windows_pe_only() {
        let xml = only(|options| options.bypass_requirements = true);

        assert!(xml.contains("<settings pass=\"windowsPE\">"));
        for value in ["BypassTPMCheck", "BypassSecureBootCheck", "BypassRAMCheck"] {
            assert!(xml.contains(&format!(
                r"reg add HKLM\SYSTEM\Setup\LabConfig /v {value} /t REG_DWORD /d 1 /f"
            )));
        }
        assert!(!xml.contains("specialize") && !xml.contains("oobeSystem"));
    }

    #[test]
    fn creates_a_local_account_without_writing_a_password() {
        let xml = only(|options| options.local_account = Some("alex".to_string()));

        assert!(xml.contains("<Name>alex</Name>"));
        assert!(xml.contains("<Group>Administrators</Group>"));
        assert!(xml.contains("BypassNRO"));
        assert!(xml.contains("<HideOnlineAccountScreens>true</HideOnlineAccountScreens>"));
        assert!(xml.contains("net user &quot;alex&quot; /logonpasswordchg:yes"));
        assert!(xml.contains("<LogonCount>1</LogonCount>"));
        // Every password in the file is empty.
        assert_eq!(xml.matches("<Password>").count(), 2);
        assert_eq!(xml.matches("<Value></Value>").count(), 2);
        assert!(!xml.contains("ProtectYourPC"));
    }

    #[test]
    fn skips_the_privacy_questions() {
        let xml = only(|options| options.skip_privacy_questions = true);

        assert!(xml.contains("<ProtectYourPC>3</ProtectYourPC>"));
        assert!(!xml.contains("LocalAccount") && !xml.contains("HideOnlineAccountScreens"));
    }

    #[test]
    fn turns_off_automatic_device_encryption() {
        let xml = only(|options| options.disable_bitlocker_auto_encryption = true);

        assert!(xml.contains(
            r"reg add HKLM\SYSTEM\CurrentControlSet\Control\BitLocker /v PreventDeviceEncryption /t REG_DWORD /d 1 /f"
        ));
        assert!(xml.contains("Microsoft-Windows-Deployment"));
        assert!(!xml.contains("BypassNRO"));
    }

    #[test]
    fn copies_the_region_and_keyboard() {
        let xml = only(|options| options.region = all_options().region);
        let several = only(|options| {
            options.region = Some(RegionSettings {
                input_locale: "0809:00020409;0809:00000809".to_string(),
                ..all_options().region.unwrap()
            })
        });
        assert!(several.contains("<InputLocale>0809:00020409;0809:00000809</InputLocale>"));

        assert!(xml.contains("<InputLocale>0809:00000809</InputLocale>"));
        assert!(xml.contains("<SystemLocale>en-GB</SystemLocale>"));
        assert!(xml.contains("<UserLocale>en-GB</UserLocale>"));
        assert!(!xml.contains("UILanguage"));
    }

    #[test]
    fn names_the_isos_architecture_in_every_component() {
        let xml = answer_file(&all_options(), Architecture::Arm64).unwrap();

        let components = xml.matches("<component ").count();
        // Setup in Windows PE, Deployment, International-Core and Shell-Setup.
        assert_eq!(components, 4);
        assert_eq!(
            xml.matches("processorArchitecture=\"arm64\"").count(),
            components
        );
        assert_eq!(
            Architecture::from_uefi_loader("efi/boot/BOOTX64.EFI"),
            Some(Architecture::Amd64)
        );
        assert_eq!(
            Architecture::from_uefi_loader("efi/boot/bootaa64.efi"),
            Some(Architecture::Arm64)
        );
        assert_eq!(
            Architecture::from_uefi_loader("efi/boot/bootriscv64.efi"),
            None
        );
    }

    #[test]
    fn is_the_same_bytes_every_time() {
        let options = all_options();
        assert_eq!(
            answer_file(&options, Architecture::Amd64),
            answer_file(&options, Architecture::Amd64)
        );
    }

    #[test]
    fn is_well_formed_xml() {
        // xmllint checks the file independently; where it is missing the
        // test says so and passes.
        if !crate::test_support::tool_exists("xmllint") {
            eprintln!("skipping: xmllint is not installed");
            return;
        }
        let mut options = all_options();
        options.local_account = Some("Zoë O.Brien-Smith".to_string());
        let directory = crate::test_support::temp_dir("answer-file");
        let path = directory.join("autounattend.xml");
        std::fs::write(&path, answer_file(&options, Architecture::Amd64).unwrap()).unwrap();

        let output = std::process::Command::new("xmllint")
            .arg("--noout")
            .arg(&path)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&directory);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn checks_local_account_names() {
        for good in [
            "alex",
            "alex.k",
            "Zoë",
            "test user",
            "a_b-c",
            "12345678901234567890",
        ] {
            assert!(validate_account_name(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "123456789012345678901",
            "Administrator",
            "guest",
            "a\"b",
            "a&b",
            "a%b",
            "a\\b",
            "a@b",
            " lead",
            "trail ",
            "dot.",
        ] {
            assert!(validate_account_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn checks_region_values() {
        let mut options = all_options();
        assert!(options.validate().is_ok());
        options.region.as_mut().unwrap().input_locale = "0809-00000809".to_string();
        assert!(options.validate().is_err());
        options.region.as_mut().unwrap().input_locale = "0809:00020409;".to_string();
        assert!(options.validate().is_err());
        options.region.as_mut().unwrap().input_locale = "0809:00020409;0809:00000809".to_string();
        assert!(options.validate().is_ok());
        let mut options = all_options();
        options.region.as_mut().unwrap().user_locale = "en GB".to_string();
        assert!(options.validate().is_err());
    }

    #[test]
    fn describes_each_option_for_the_plan_and_the_audit_log() {
        let options = all_options();

        assert_eq!(options.describe().len(), 5);
        assert_eq!(
            options.audit_summary(),
            "bypass-requirements, local-account=alex, skip-privacy-questions, no-bitlocker-auto-encryption, region=en-GB/en-GB/0809:00000809"
        );
    }
}
