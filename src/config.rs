use std::fmt::{Display, Formatter};
use std::fs::read_to_string;
use serde::{Deserialize, Deserializer};
use linked_hash_map::LinkedHashMap;
use poise::serenity_prelude::ReactionType;
use serde::de::{Error, Visitor};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
	pub bot_token: String,
	pub mongodb: String,
	pub welcome: FileReference,
	pub self_managment: SelfManagement,
	pub toc: Vec<TableOfContentEntry>,
	pub self_assignments: SelfAssignments,
	pub assignments: LinkedHashMap<String, Assignment>,
}

#[derive(Debug)]
pub struct FileReference {
	pub filename: String,
	pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelfManagement {
	pub category: u64,
	pub ownership: bool,
	pub limit: u64,
	pub make_channel_admin: bool,
	pub abandon_after: u64,
	pub claiming: bool,
	pub logging: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelfAssignments {
	pub label: String,
	#[serde(with = "serde_with::rust::display_fromstr")]
	pub icon: ReactionType,
	pub prolog: FileReference,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableOfContentEntry {
	pub label: String,
	#[serde(with = "serde_with::rust::display_fromstr")]
	pub icon: ReactionType,
	pub file: FileReference,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
	pub title: String,
	pub roles: Vec<Role>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
	pub label: String,
	pub subscript: Option<String>,
	#[serde(with = "serde_with::rust::display_fromstr")]
	pub icon: ReactionType,
	pub role: u64,
}

impl Display for FileReference {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", &self.content)
	}
}

impl<'a> From<&'a FileReference> for &'a str {

	fn from(f: &'a FileReference) -> Self {
		f.content.as_str()
	}
}

impl<'de> Deserialize<'de> for FileReference {
	fn deserialize<D>(
		deserializer: D
	) -> Result<Self, D::Error> where
		D: Deserializer<'de>,
	{
		struct FilenameVisitor;
		impl<'de> Visitor<'de> for FilenameVisitor {
			type Value = FileReference;

			fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
				formatter.write_str("path to readable file")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> where E: Error {
				self.visit_string(v.to_owned())
			}

			fn visit_string<E>(self, filename: String) -> Result<Self::Value, E> where E: Error {
				let content = read_to_string(&filename).map_err(|err| {
					Error::custom(format!("file {} could not be read: {}", &filename, err))
				})?;

				Ok(FileReference {
					filename,
					content,
				})
			}
		}

		deserializer.deserialize_string(FilenameVisitor)
	}
}
