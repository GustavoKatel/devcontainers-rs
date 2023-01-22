use bollard::service::{Mount, MountTypeEnum};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context as AnyhowContext, Result};

pub trait MountExt: Sized {
    fn from_comma_string(s: &str) -> Result<Self>;
    fn from_colon_string(s: &str) -> Result<Self>;

    fn parse_from_str(s: &str) -> Result<Self>;
}

impl MountExt for Mount {
    fn from_colon_string(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(":").collect();

        if parts.len() < 2 {
            bail!("Invalid mount point: {}", s);
        }

        Ok(Mount {
            source: Some(parts[0].to_string()),
            target: Some(parts[1].to_string()),
            typ: Some(MountTypeEnum::BIND),
            ..Mount::default()
        })
    }

    fn from_comma_string(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(",").collect();

        if parts.len() == 0 {
            bail!("Invalid mount point");
        }

        let mut mount = Mount::default();

        for part in parts {
            let attr_parts: Vec<&str> = part.split("=").collect();
            if attr_parts.len() < 2 {
                bail!("Invalid mount point: {}", s);
            }

            let attr_name = attr_parts[0];
            let attr_value = attr_parts[1];

            match attr_name {
                "source" => {
                    mount.source = Some(attr_value.to_string());
                }
                "target" => {
                    mount.target = Some(attr_value.to_string());
                }
                "type" => {
                    mount.typ = Some(MountTypeEnum::from_str(attr_value).map_err(|err| {
                        anyhow!("Invalid mount point type: {} {}", attr_value, err)
                    })?);
                }
                "consistency" => {
                    mount.consistency = Some(attr_value.to_string());
                }
                attr => {
                    bail!("Invalid attr '{}' for mount point: {}", attr, s)
                }
            };
        }

        Ok(mount)
    }

    fn parse_from_str(s: &str) -> Result<Self> {
        if s.contains(",") {
            Self::from_comma_string(s)
        } else {
            Self::from_colon_string(s)
        }
    }
}
