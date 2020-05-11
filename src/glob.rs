use std::cmp::Ordering;
use std::fmt;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use glob::Pattern;
use unicase::UniCase;

#[derive(Clone, PartialEq)]
pub enum GlobType {
    Literal(String),
    Simple(String),
    Full(Pattern),
}

impl Eq for GlobType {}

impl fmt::Debug for GlobType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GlobType::Literal(name) => write!(f, "Literal '{}'", name),
            GlobType::Simple(pattern) => write!(f, "Simple glob '*{}'", pattern),
            GlobType::Full(pattern) => write!(f, "Full glob '{}'", pattern),
        }
    }
}

fn determine_type(glob: &str) -> GlobType {
    let mut maybe_simple = false;

    for (idx, ch) in glob.bytes().enumerate() {
        if idx == 0 && ch == b'*' {
            maybe_simple = true;
        } else if ch == b'\\' || ch == b'[' || ch == b'*' || ch == b'?' {
            return GlobType::Full(Pattern::new(&glob).unwrap());
        }
    }

    if maybe_simple {
        GlobType::Simple(glob[1..].to_string())
    } else {
        GlobType::Literal(glob.to_string())
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Glob {
    glob: GlobType,
    weight: i32,
    case_sensitive: bool,
    mime_type: String,
}

impl fmt::Debug for Glob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Glob: {:?} {:?} (weight: {}, cs: {})",
            self.glob, self.mime_type, self.weight, self.case_sensitive
        )
    }
}

impl Ord for Glob {
    fn cmp(&self, other: &Glob) -> Ordering {
        self.weight.cmp(&other.weight)
    }
}

impl PartialOrd for Glob {
    fn partial_cmp(&self, other: &Glob) -> Option<Ordering> {
        Some(self.weight.cmp(&other.weight))
    }
}

impl Glob {
    pub fn simple(mime_type: &str, glob: &str) -> Glob {
        Glob {
            mime_type: mime_type.to_string(),
            glob: determine_type(glob),
            weight: 50,
            case_sensitive: false,
        }
    }

    pub fn with_weight(mime_type: &str, glob: &str, weight: i32) -> Glob {
        Glob {
            mime_type: mime_type.to_string(),
            glob: determine_type(glob),
            weight,
            case_sensitive: false,
        }
    }

    pub fn new(mime_type: &str, glob: &str, weight: i32, cs: bool) -> Glob {
        Glob {
            mime_type: mime_type.to_string(),
            glob: determine_type(glob),
            weight,
            case_sensitive: cs,
        }
    }

    pub fn from_v1_string(s: &str) -> Option<Glob> {
        let mut chunks = s.split(':').fuse();
        let mime_type = chunks.next().filter(|s| !s.is_empty())?;
        let glob = chunks.next().filter(|s| !s.is_empty())?;

        // The globs file is not extensible, so consume any
        // leftover tokens
        if chunks.next().is_some() {
            return None;
        }

        Some(Glob::new(mime_type, glob, 50, false))
    }

    pub fn from_v2_string(s: &str) -> Option<Glob> {
        let mut chunks = s.split(':').fuse();

        let weight = chunks
            .next()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|n| *n >= 0)?;

        let mime_type = chunks.next()?;
        let glob = chunks.next()?;

        let mut case_sensitive = false;
        if let Some(flags) = chunks.next() {
            let flags_chunks = flags.split(',').collect::<Vec<&str>>();

            // Allow for extra flags
            if flags_chunks.iter().position(|&f| f == "cs").is_some() {
                case_sensitive = true;
            }
        }

        // Ignore any other token, for extensibility:
        //
        // https://specifications.freedesktop.org/shared-mime-info-spec/shared-mime-info-spec-latest.html#idm46152099256048

        Some(Glob::new(mime_type, glob, weight, case_sensitive))
    }

    fn compare(&self, file_name: &str) -> bool {
        match &self.glob {
            GlobType::Literal(s) => {
                let a = UniCase::new(s);
                let b = UniCase::new(file_name);

                return a == b;
            }
            GlobType::Simple(s) => {
                if file_name.ends_with(s) {
                    return true;
                }

                if !self.case_sensitive {
                    let lc_file_name = file_name.to_lowercase();
                    if lc_file_name.ends_with(s) {
                        return true;
                    }
                }
            }
            GlobType::Full(p) => {
                return p.matches(file_name);
            }
        }

        false
    }
}

pub fn read_globs_v1_from_file<P: AsRef<Path>>(file_name: P) -> Option<Vec<Glob>> {
    let f = match File::open(file_name) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let mut res = Vec::new();
    let file = BufReader::new(&f);
    for line in file.lines() {
        if let Err(_) = line {
            return None;
        }

        let line = line.unwrap();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        match Glob::from_v1_string(&line) {
            Some(v) => res.push(v),
            None => continue,
        }
    }

    Some(res)
}

pub fn read_globs_v2_from_file<P: AsRef<Path>>(file_name: P) -> Option<Vec<Glob>> {
    let f = match File::open(file_name) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let mut res = Vec::new();
    let file = BufReader::new(&f);
    for line in file.lines() {
        if let Err(_) = line {
            return None;
        }

        let line = line.unwrap();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        match Glob::from_v2_string(&line) {
            Some(v) => res.push(v),
            None => continue,
        }
    }

    Some(res)
}

pub fn read_globs_from_dir<P: AsRef<Path>>(dir: P) -> Vec<Glob> {
    let mut globs_file = PathBuf::new();
    globs_file.push(dir);
    globs_file.push("globs2");

    match read_globs_v2_from_file(&globs_file) {
        Some(v) => v,
        None => {
            globs_file.pop();
            globs_file.push("globs");

            read_globs_v1_from_file(globs_file).unwrap_or_default()
        }
    }
}

pub struct GlobMap {
    globs: Vec<Glob>,
}

impl GlobMap {
    pub fn new() -> GlobMap {
        GlobMap { globs: Vec::new() }
    }

    pub fn add_glob(&mut self, glob: Glob) {
        self.globs.push(glob);
    }

    pub fn add_globs(&mut self, globs: &[Glob]) {
        self.globs.extend_from_slice(globs);
    }

    pub fn lookup_mime_type_for_file_name(&self, file_name: &str) -> Option<Vec<String>> {
        let mut matching_globs = Vec::new();

        for glob in &self.globs {
            if glob.compare(file_name) {
                matching_globs.push(glob.clone());
            }
        }

        if matching_globs.is_empty() {
            return None;
        }

        matching_globs.sort();

        let res = matching_globs
            .iter()
            .map(|glob| glob.mime_type.clone())
            .collect();

        Some(res)
    }
}

impl fmt::Debug for GlobMap {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut lines = String::new();
        for glob in &self.globs {
            lines.push_str(&format!("{:?}", glob));
            lines.push_str("\n");
        }

        write!(f, "Globs:\n{}", lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_type() {
        assert_eq!(
            determine_type("*.gif"),
            GlobType::Simple(".gif".to_string())
        );
        assert_eq!(
            determine_type("Foo*.gif"),
            GlobType::Full(Pattern::new("Foo*.gif").unwrap())
        );
        assert_eq!(
            determine_type("*[4].gif"),
            GlobType::Full(Pattern::new("*[4].gif").unwrap())
        );
        assert_eq!(
            determine_type("Makefile"),
            GlobType::Literal("Makefile".to_string())
        );
        assert_eq!(
            determine_type("sldkfjvlsdf\\\\slkdjf"),
            GlobType::Full(Pattern::new("sldkfjvlsdf\\\\slkdjf").unwrap())
        );
        assert_eq!(
            determine_type("tree.[ch]"),
            GlobType::Full(Pattern::new("tree.[ch]").unwrap())
        );
    }

    #[test]
    fn glob_v1_string() {
        assert_eq!(
            Glob::from_v1_string("text/rust:*.rs"),
            Some(Glob::simple("text/rust", "*.rs"))
        );
        assert_eq!(
            Glob::from_v1_string("text/rust:*.rs"),
            Some(Glob::new("text/rust", "*.rs", 50, false))
        );

        assert_eq!(Glob::from_v1_string(""), None);
        assert_eq!(Glob::from_v1_string("foo"), None);
        assert_eq!(Glob::from_v1_string("foo:"), None);
        assert_eq!(Glob::from_v1_string(":bar"), None);
        assert_eq!(Glob::from_v1_string(":"), None);
        assert_eq!(Glob::from_v1_string("foo:bar:baz"), None);
    }

    #[test]
    fn glob_v2_string() {
        assert_eq!(
            Glob::from_v2_string("80:text/rust:*.rs"),
            Some(Glob::with_weight("text/rust", "*.rs", 80))
        );
        assert_eq!(
            Glob::from_v2_string("80:text/rust:*.rs"),
            Some(Glob::new("text/rust", "*.rs", 80, false))
        );
        assert_eq!(
            Glob::from_v2_string("50:text/x-c++src:*.C:cs"),
            Some(Glob::new("text/x-c++src", "*.C", 50, true))
        );

        assert_eq!(Glob::from_v2_string(""), None);
        assert_eq!(Glob::from_v2_string("foo"), None);
        assert_eq!(Glob::from_v2_string("foo:"), None);
        assert_eq!(Glob::from_v2_string(":bar"), None);
        assert_eq!(Glob::from_v2_string(":"), None);
        assert_eq!(Glob::from_v2_string("foo:bar:baz"), None);
        assert_eq!(Glob::from_v2_string("foo:bar:baz:blah"), None);

        assert_eq!(
            Glob::from_v2_string("50:text/x-c++src:*.C:cs,newflag:newfeature:somethingelse"),
            Some(Glob::new("text/x-c++src", "*.C", 50, true))
        );
    }

    #[test]
    fn compare() {
        // Literal
        let copying = Glob::new("text/x-copying", "copying", 50, false);
        assert_eq!(copying.compare(&"COPYING".to_string()), true);

        // Simple, case-insensitive
        let c_src = Glob::new("text/x-csrc", "*.c", 50, false);
        assert_eq!(c_src.compare(&"foo.c".to_string()), true);
        assert_eq!(c_src.compare(&"FOO.C".to_string()), true);

        // Simple, case-sensitive
        let cplusplus_src = Glob::new("text/x-c++src", "*.C", 50, true);
        assert_eq!(cplusplus_src.compare(&"foo.C".to_string()), true);
        assert_eq!(cplusplus_src.compare(&"foo.c".to_string()), false);
        assert_eq!(cplusplus_src.compare(&"foo.h".to_string()), false);

        // Full
        let video_x_anim = Glob::new("video/x-anim", "*.anim[1-9j]", 50, false);
        assert_eq!(video_x_anim.compare(&"foo.anim0".to_string()), false);
        assert_eq!(video_x_anim.compare(&"foo.anim8".to_string()), true);
        assert_eq!(video_x_anim.compare(&"foo.animk".to_string()), false);
        assert_eq!(video_x_anim.compare(&"foo.animj".to_string()), true);
    }
}
