//! Pure setup choices and validation; credentials never belong in this form.

use super::{MicrosoftClient, MicrosoftError, guid};

pub const MICROSOFT_SETUP_URL: &str =
    "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade";
pub const MICROSOFT_SETUP_HELP: &str =
    "https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app";
pub const RETURN_ADDRESS: &str = "http://localhost";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MicrosoftSetupKind {
    #[default]
    Work,
    Personal,
}

impl MicrosoftSetupKind {
    pub fn account_choice(self) -> &'static str {
        match self {
            Self::Work => "Single tenant only (Accounts in this organizational directory only)",
            Self::Personal => "Personal accounts only (Personal Microsoft accounts)",
        }
    }

    pub fn help_request(self) -> &'static str {
        match self {
            Self::Work => {
                "Please help me connect my Microsoft 365 account to Sovra on my computer. In our Microsoft Entra directory, create an app registration for accounts in this directory only. Add Mobile and desktop applications with redirect URI http://localhost. Use delegated Microsoft Graph permissions: User.Read, offline_access, Mail.ReadWrite, Calendars.ReadWrite, Files.Read, Contacts.Read, Tasks.ReadWrite and Notes.Read. Please approve only as our policy allows. Send me the Application (client) ID and Directory (tenant) ID from Overview, not the Object ID. No client secret or password is needed. Sovra reads my content, creates email drafts for me to send, adds appointments without guests and adds to-dos when I ask. Optional work services stay off until I choose them."
            }
            Self::Personal => {
                "Please help me create a connection for my own Outlook.com or Hotmail account in Sovra. An accessible Microsoft Entra directory is needed to register the app, even though I will sign in with a personal Microsoft account. Choose Personal accounts only, and Mobile and desktop applications with redirect URI http://localhost. Send me only the Application (client) ID from Overview. Do not create or send a client secret. Microsoft's registration guide is https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app ."
            }
        }
    }
}

impl MicrosoftClient {
    pub fn from_setup(
        kind: MicrosoftSetupKind,
        client_id: &str,
        tenant_id: &str,
    ) -> Result<Self, MicrosoftError> {
        let client_id = client_id.trim().to_ascii_lowercase();
        if !setup_guid(&client_id) {
            return Err(MicrosoftError::Failed("Copy the Application (client) ID from Microsoft's Overview page. It is 36 characters with dashes. Do not paste an email address, password or secret.".into()));
        }
        let tenant = match kind {
            MicrosoftSetupKind::Personal => "consumers".to_owned(),
            MicrosoftSetupKind::Work => {
                let tenant = tenant_id.trim().to_ascii_lowercase();
                if !setup_guid(&tenant) {
                    return Err(MicrosoftError::Failed("Copy the Directory (tenant) ID from the same Overview page. It is 36 characters with dashes.".into()));
                }
                if tenant == client_id {
                    return Err(MicrosoftError::Failed("The two IDs should be different. Copy Application (client) ID and Directory (tenant) ID, not Object ID.".into()));
                }
                tenant
            }
        };
        Ok(Self { client_id, tenant })
    }
}

fn setup_guid(value: &str) -> bool {
    guid(value) && value.bytes().any(|byte| byte != b'0' && byte != b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    const APP: &str = "1aaabbbb-0000-4000-8000-000000000001";
    const TENANT: &str = "2aaabbbb-0000-4000-8000-000000000002";

    #[test]
    fn work_setup_requires_two_distinct_public_ids_and_normalizes_paste() {
        let config = MicrosoftClient::from_setup(
            MicrosoftSetupKind::Work,
            &format!(" {} \n", APP.to_uppercase()),
            TENANT,
        )
        .unwrap();
        assert_eq!(config.client_id, APP);
        assert_eq!(config.tenant, TENANT);
        assert!(config.validate().is_ok());
        for (app, tenant) in [
            (APP, APP),
            (APP, "common"),
            (APP, ""),
            ("secret", TENANT),
            ("00000000-0000-0000-0000-000000000000", TENANT),
        ] {
            assert!(MicrosoftClient::from_setup(MicrosoftSetupKind::Work, app, tenant).is_err());
        }
    }

    #[test]
    fn personal_setup_uses_consumers_and_never_needs_a_directory_code() {
        let config =
            MicrosoftClient::from_setup(MicrosoftSetupKind::Personal, APP, "unused").unwrap();
        assert_eq!(config.tenant, "consumers");
        assert!(
            MicrosoftSetupKind::Personal
                .account_choice()
                .contains("Personal")
        );
        assert!(
            MicrosoftSetupKind::Work
                .account_choice()
                .contains("Single tenant")
        );
        for kind in [MicrosoftSetupKind::Personal, MicrosoftSetupKind::Work] {
            assert!(kind.help_request().contains(RETURN_ADDRESS));
            assert!(kind.help_request().contains("Application (client) ID"));
        }
    }
}
