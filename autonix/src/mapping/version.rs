use std::{iter::Peekable, str::FromStr};
use serde::Serialize;


#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Version {
    SemVer(SemVerVersion),
    Multiple(Vec<Version>),
    Docker(String),
    Unknown(Option<String>),
}

impl Version {
    //TOOD: avoid converting to String, again
    pub fn to_nix(&self)->String {
        match self {
            Version::SemVer(semver) => format!("{:?}", semver),
            Version::Docker(docker_str) => docker_str.to_owned(),
            //TODO: wtf is this shit? AI generated buillshit to delete !!
            Version::Multiple(versions) => {
                let mut versions_str = String::from("[ ");
                for v in versions {
                    versions_str.push_str(&v.to_nix());
                        versions_str.push(' ');
                }
                versions_str.push(']');
                versions_str
            }
            Version::Unknown(_) => "latest".to_owned(),
        }
    }
}

impl From<Option<&str>> for Version {
    fn from(s: Option<&str>) -> Self {
        Self::Unknown(s.map(|s| s.to_string()))
    }
}

impl Default for Version {
    fn default() -> Self {
        Self::Unknown(None)
    }
}


//TODO: remove the clone

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreRelease(Vec<String>);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Build(Vec<String>);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Core {
    major: u16,
    minor: u16,
    patch: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum SemVerVersion {
    ValidCore(Core),
    PreRelease {
        core: Core,
        pre_release: PreRelease,
    },
    Build {
        core: Core,
        build: Build,
    },
    PreReleaseAndBuild {
        core: Core,
        pre_release: PreRelease,
        build: Build,
    },
    Unknown(String),
}

impl std::fmt::Display for SemVerVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SemVerVersion::ValidCore(core) => {
                write!(f, "{}.{}.{}", core.major, core.minor, core.patch)
            }
            SemVerVersion::PreRelease { core, pre_release } => {
                write!(
                    f,
                    "{}.{}.{}-{}",
                    core.major,
                    core.minor,
                    core.patch,
                    pre_release.0.join(".")
                )
            }
            SemVerVersion::Build { core, build } => {
                write!(
                    f,
                    "{}.{}.{}+{}",
                    core.major,
                    core.minor,
                    core.patch,
                    build.0.join(".")
                )
            }
            SemVerVersion::PreReleaseAndBuild {
                core,
                pre_release,
                build,
            } => {
                write!(
                    f,
                    "{}.{}.{}-{}+{}",
                    core.major,
                    core.minor,
                    core.patch,
                    pre_release.0.join("."),
                    build.0.join(".")
                )
            }
            SemVerVersion::Unknown(s) => write!(f, "{}", s),
        }
    }
}

impl From<String> for SemVerVersion {
    fn from(s: String) -> Self {
        Self::from_str(&s).unwrap_or(SemVerVersion::Unknown(s))
    }
}

impl FromStr for SemVerVersion {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let chars = s.chars().peekable();

        Self::valid_semver(chars)
    }
}

impl SemVerParser for SemVerVersion {}

#[derive(Debug, PartialEq)]
pub enum Error {
    InvalidFormat,
    LeadingZero,
    TrailingCharacters,
}

//NOTE: first let's just parse one version, then once the parser for versions works, we'll move on to parse multiple versions in the same like using operators like || and so on
pub trait SemVerParser {
    /// <valid semver> ::= <version core> | <version core> "-" <pre-release> | <version core> "+" <build> | <version core> "-" <pre-release> "+" <build>
    fn valid_semver(mut input: Peekable<std::str::Chars<'_>>) -> Result<SemVerVersion, Error>
    where
        Self: Sized,
    {
        //TODO: check that the first char isn't some symbol
        let core = Self::version_core(&mut input)?;

        match input.next() {
            None => Ok(SemVerVersion::ValidCore(core)),
            Some('-') => {
                let pre_release = Self::pre_release(&mut input)?;
                match input.next() {
                    None => Ok(SemVerVersion::PreRelease { core, pre_release }),
                    Some('+') => {
                        let build = Self::build(&mut input)?;
                        Ok(SemVerVersion::PreReleaseAndBuild {
                            core,
                            pre_release,
                            build,
                        })
                    }
                    Some(_) => Err(Error::TrailingCharacters),
                }
            }
            Some('+') => Self::build(&mut input).map(|build| SemVerVersion::Build { core, build }),
            Some(_) => Err(Error::InvalidFormat),
        }
    }

    /// <version core> ::= <major> "." <minor> "." <patch>
    /// <major>, <minor>, <patch> ::= <numeric identifier>
    fn version_core(input: &mut Peekable<std::str::Chars<'_>>) -> Result<Core, Error> {
        // <major> ::= <numeric identifier>
        let major = Self::numeric_identifier(input)?;
        if input.next().is_some_and(|c| c != '.') {
            return Err(Error::InvalidFormat);
        }

        // <minor> ::= <numeric identifier>
        let minor = Self::numeric_identifier(input)?;
        if input.next().is_some_and(|c| c != '.') {
            return Err(Error::InvalidFormat);
        }

        // <patch> ::= <numeric identifier>
        let patch = Self::numeric_identifier(input)?;

        Ok(Core {
            major,
            minor,
            patch,
        })
    }

    /// <numeric identifier> ::= "0" | <positive digit> | <positive digit> <digits>
    /// <digits> ::= <digit> | <digit> <digits>
    /// <digit> ::= "0" | <positive digit>
    /// <positive digit> ::= "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
    fn numeric_identifier(input: &mut Peekable<std::str::Chars<'_>>) -> Result<u16, Error> {
        let first_digit = input.next().ok_or(Error::InvalidFormat)?;
        let next_digit = input.peek();

        match (first_digit, next_digit) {
            // '0' followed by another digit is invalid
            ('0', Some(&ch)) if ch.is_digit(10) => Err(Error::LeadingZero),
            // '0' alone is valid
            ('0', _) => return Ok(0),
            // Positive digit case
            (d, _) if d.is_digit(10) => {
                let mut major_str = String::new();
                major_str.push(d);

                while let Some(ch) = input.next_if(|c| c.is_digit(10)) {
                    major_str.push(ch);
                }

                major_str.parse::<u16>().map_err(|_| Error::InvalidFormat)
            }
            _ => Err(Error::InvalidFormat),
        }
    }

    /// <pre-release> ::= <dot-separated pre-release identifiers>
    /// <dot-separated pre-release identifiers> ::= <pre-release identifier> | <pre-release identifier> "." <dot-separated pre-release identifiers>
    /// <pre-release identifier> ::= <alphanumeric identifier> | <numeric identifier>
    fn pre_release(input: &mut Peekable<std::str::Chars<'_>>) -> Result<PreRelease, Error> {
        Self::dot_separated_identifiers(input).map(PreRelease)
    }

    /// <build> ::= <dot-separated build identifiers>
    /// <dot-separated build identifiers> ::= <build identifier> | <build identifier> "." <dot-separated build identifiers>
    /// <build identifier> ::= <alphanumeric identifier> | <digits>
    fn build(input: &mut Peekable<std::str::Chars<'_>>) -> Result<Build, Error> {
        let build = Self::dot_separated_identifiers(input).map(Build)?;
        // When parsing the build, we should be at the end of the version everytime
        if input.next().is_none() {
            Ok(build)
        } else {
            // TODO: do we want to raise an error or ignore?
            Err(Error::TrailingCharacters)
        }
    }

    /// This can be either an alphanumeric identifier or a numeric identifier, therefore instead of using the numeric_identifier function directly,
    /// we'll just use the alphanumeric_identifier function to parse both cases, then try to convert to a number if possible (do we need it as a number?)
    /// NOTE: This is the same logic for the build and the pre_release identifier, for both cases we use a String, let's see if in the future we want an enum or something to differentiate between digit and alphanumeric
    fn dot_separated_identifiers(
        input: &mut Peekable<std::str::Chars<'_>>,
    ) -> Result<Vec<String>, Error> {
        let mut identifiers = Vec::new();
        let identifier = Self::alphanumeric_identifier(input)?;
        identifiers.push(identifier);
        // Continue parsing while there are more dot-separated identifiers
        while let Some(_) = input.next_if(|&ch| ch == '.') {
            let identifier = Self::alphanumeric_identifier(input)?;
            identifiers.push(identifier);
        }
        Ok(identifiers)
    }

    /// <alphanumeric identifier> ::= <non-digit> | <non-digit> <identifier characters> | <identifier characters> <non-digit> | <identifier characters> <non-digit> <identifier characters>
    /// <non-digit> ::= <letter> | "-"
    /// <letter> ::= "a-z" | "A-Z" (shortened for clarity)
    /// <identifier characters> ::= <identifier character> | <identifier character> <identifier characters>
    /// <identifier character> ::= <digit> | <non-digit>
    fn alphanumeric_identifier(input: &mut Peekable<std::str::Chars<'_>>) -> Result<String, Error> {
        let mut identifier = String::new();

        while let Some(ch) = input.next_if(|ch| {
            matches!(ch,
            //digit
            '0'..='9' |
            //non-digit
            'a'..='z' |
            'A'..='Z' |
            '-')
        }) {
            identifier.push(ch);
        }

        if identifier.is_empty() {
            Err(Error::InvalidFormat)
        } else {
            Ok(identifier)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_identifier() {
        let mut input = "1230".chars().peekable();
        let result = SemVerVersion::numeric_identifier(&mut input).unwrap();
        assert_eq!(result, 1230);

        let mut input = "0".chars().peekable();
        let result = SemVerVersion::numeric_identifier(&mut input).unwrap();
        assert_eq!(result, 0);

        let mut input = "01".chars().peekable();
        let result = SemVerVersion::numeric_identifier(&mut input);
        assert!(matches!(result, Err(Error::LeadingZero)));

        let mut input = "abc".chars().peekable();
        let result = SemVerVersion::numeric_identifier(&mut input);
        assert!(matches!(result, Err(Error::InvalidFormat)));

        // This will only parse the first two digits, we currently only loop and check if the next char is a digit
        // The error will be raised after when we try to check that there is a dot. Should we fail faster?
        let mut input = "12 4".chars().peekable();
        let result = SemVerVersion::numeric_identifier(&mut input);
        assert_eq!(result, Ok(12));
    }

    #[test]
    fn test_version_core() {
        let mut input = "1.2.3".chars().peekable();
        let core = SemVerVersion::version_core(&mut input).unwrap();
        assert_eq!(core.major, 1);
        assert_eq!(core.minor, 2);
        assert_eq!(core.patch, 3);

        let mut input = "1.2".chars().peekable();
        let result = SemVerVersion::version_core(&mut input);
        assert!(matches!(result, Err(Error::InvalidFormat)));
    }

    #[test]
    fn test_dot_separated_identifiers() {
        let mut input = "alpha.1.beta".chars().peekable();
        let pre_release = SemVerVersion::dot_separated_identifiers(&mut input).unwrap();
        assert_eq!(
            pre_release,
            vec!["alpha".to_string(), "1".to_string(), "beta".to_string()]
        );

        let mut input = "alpha..beta".chars().peekable();
        let result = SemVerVersion::dot_separated_identifiers(&mut input);
        assert!(matches!(result, Err(Error::InvalidFormat)));

        let mut input = "".chars().peekable();
        let result = SemVerVersion::dot_separated_identifiers(&mut input);
        assert!(matches!(result, Err(Error::InvalidFormat)));

        let mut input = "01.beta".chars().peekable();
        let pre_release = SemVerVersion::dot_separated_identifiers(&mut input).unwrap();
        assert_eq!(pre_release, vec!["01".to_string(), "beta".to_string()]);
    }

    #[test]
    fn test_simple_version() {
        let SemVerVersion::ValidCore(v) = SemVerVersion::from_str("1.2.3").unwrap() else {
            panic!("Expected ValidCore")
        };
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_version_with_pre_release() {
        let SemVerVersion::PreRelease { core, pre_release } =
            SemVerVersion::from_str("1.2.3-alpha").unwrap()
        else {
            panic!("Expected PreRelease")
        };
        assert_eq!(core.major, 1);
        assert_eq!(core.minor, 2);
        assert_eq!(core.patch, 3);
        assert_eq!(pre_release.0, vec!["alpha".to_string()]);
    }

    #[test]
    fn test_leading_zero_rejected() {
        assert!(SemVerVersion::from_str("01.2.3").is_err());
        assert!(SemVerVersion::from_str("1.02.3").is_err());
        assert!(SemVerVersion::from_str("1.2.03").is_err());
    }

    /// NOTE: should this be valid or invalid? I think that valid because we have an alphanumeric identifier here, not a numeric one
    #[test]
    fn test_valid_pre_release_leading_zero() {
        let SemVerVersion::PreRelease { core, pre_release } =
            SemVerVersion::from_str("1.2.3-01").unwrap()
        else {
            panic!("Expected PreRelease")
        };
        assert_eq!(core.major, 1);
        assert_eq!(core.minor, 2);
        assert_eq!(core.patch, 3);
        assert_eq!(pre_release.0, vec!["01".to_string()]);
    }

    #[test]
    fn test_trailing_characters_rejected() {
        assert!(SemVerVersion::from_str("1.2.3xyz").is_err());
    }

    #[test]
    fn test_version_with_build() {
        let SemVerVersion::Build { core, build } =
            SemVerVersion::from_str("1.2.3+001").unwrap()
        else {
            panic!("Expected Build")
        };
        assert_eq!(core.major, 1);
        assert_eq!(core.minor, 2);
        assert_eq!(core.patch, 3);
        assert_eq!(build.0, vec!["001".to_string()]);
    }

    #[test]
    fn test_version_with_pre_release_and_build() {
        let SemVerVersion::PreReleaseAndBuild { core, pre_release, build } =
            SemVerVersion::from_str("1.2.3-alpha.1+001").unwrap()
        else {
            panic!("Expected PreReleaseAndBuild")
        };
        assert_eq!(core.major, 1);
        assert_eq!(core.minor, 2);
        assert_eq!(core.patch, 3);
        assert_eq!(pre_release.0, vec!["alpha".to_string(), "1".to_string()]);
        assert_eq!(build.0, vec!["001".to_string()]);
    }
}
