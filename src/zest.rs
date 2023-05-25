use pulldown_cmark::{Event, HeadingLevel, Parser, Tag};
use serde::Deserialize;
use std::error::Error;
use std::fmt::Display;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Debug)]
pub enum ZestParsingError {
    SourceError(std::io::Error),
    MetadataError(String),
}

impl Display for ZestParsingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceError(e) => e.fmt(f),
            Self::MetadataError(s) => write!(f, "Error while parsing metadata: {}", s),
        }
    }
}

impl Error for ZestParsingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SourceError(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Deserialize, Debug, Default, Clone)]
pub struct ZestMeta {
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Zest {
    pub title: String,
    pub content: String,
    pub file: String,
    pub refs: Vec<String>,
    pub metadata: ZestMeta,
}

impl Zest {
    fn new(
        title: String,
        content: String,
        file: String,
        refs: Vec<String>,
        metadata: ZestMeta,
    ) -> Self {
        Zest {
            title,
            content,
            file,
            refs,
            metadata,
        }
    }

    pub fn from_file(source: String) -> Result<Self, ZestParsingError> {
        // TODO(vigoux): not really optimal because there's a lot of allocations, but that should
        // not happen very often...

        // Split the file in two parts: the metadata part (in a yaml header, if any) and the
        // markdown lines.

        let file = File::open(&source).map_err(|e| ZestParsingError::SourceError(e))?;
        let reader = BufReader::new(file);
        let mut metadata = String::new();
        let mut md_lines = String::new();

        let mut in_header = false;
        for (i, line) in reader.lines().filter_map(|l| l.ok()).enumerate() {
            match (i, line.as_ref(), in_header) {
                (0, "---", false) => {
                    in_header = true;
                }
                (_, "---", true) => {
                    in_header = false;
                }
                (_, l, true) => {
                    if !metadata.is_empty() {
                        metadata.push('\n');
                    }
                    metadata.push_str(l);
                }
                (_, l, false) => {
                    if !md_lines.is_empty() {
                        md_lines.push('\n');
                    }
                    md_lines.push_str(l);
                }
            }
        }

        let mut title = String::new();
        let mut content = String::new();
        let mut refs = Vec::new();

        // Now that we've split it, parse the markdown first
        // to extract the text's content
        let mut in_title = false;
        for evt in Parser::new(md_lines.as_ref()) {
            match (in_title, evt) {
                // title handling
                (false, Event::Start(Tag::Heading(HeadingLevel::H1, _, _))) if title.is_empty() => {
                    in_title = true
                }
                (true, Event::Text(t)) => title.push_str(t.as_ref()),
                (true, Event::End(Tag::Heading(HeadingLevel::H1, _, _))) => in_title = false,

                // Normal text handling
                (false, Event::Text(t)) => content.push_str(t.as_ref()),

                // TODO(vigoux): For now we ignore the type of the link, maybe at some point we
                // will filter that and have different behaviors for this
                (false, Event::Start(Tag::Link(_, dest, text))) => {
                    content.push_str(text.as_ref());
                    refs.push(String::from(dest.as_ref()));
                }
                (true, Event::Start(Tag::Link(_, dest, text))) => {
                    title.push_str(text.as_ref());
                    refs.push(String::from(dest.as_ref()));
                }

                // Newline handling
                (
                    false,
                    Event::SoftBreak | Event::HardBreak | Event::End(Tag::Heading(_, _, _)),
                ) => content.push('\n'),

                _ => {}
            }
        }

        let metadata: ZestMeta = if !metadata.is_empty() {
            serde_yaml::from_str(metadata.as_ref())
                .map_err(|e| ZestParsingError::MetadataError(e.to_string()))?
        } else {
            ZestMeta::default()
        };

        Ok(Zest::new(title, content, source, refs, metadata))
    }
}
