use std::fmt;
use std::io;
use std::path::Path;
use std::str::FromStr;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Account {
    pub account_uuid: String,
    pub organization_uuid: String,
}

impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.account_uuid, self.organization_uuid)
    }
}

impl FromStr for Account {
    type Err = String;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        match id.split_once('/') {
            Some((account, organization)) if !account.is_empty() && !organization.is_empty() => {
                Ok(Account {
                    account_uuid: account.to_string(),
                    organization_uuid: organization.to_string(),
                })
            }
            _ => Err(format!(
                "{id:?} isn't an account ID, which is <accountUuid>/<organizationUuid>"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub account: Account,
    pub email: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawClaudeJson {
    oauth_account: Option<RawOauthAccount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOauthAccount {
    account_uuid: Option<String>,
    organization_uuid: Option<String>,
    email_address: Option<String>,
}

pub fn read_login(path: &Path) -> io::Result<Option<Login>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let raw: RawClaudeJson = serde_json::from_slice(&bytes)?;
    Ok(raw.oauth_account.and_then(|oauth| {
        Some(Login {
            account: Account {
                account_uuid: oauth.account_uuid?,
                organization_uuid: oauth.organization_uuid?,
            },
            email: oauth.email_address,
        })
    }))
}
