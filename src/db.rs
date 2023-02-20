use lazy_static::lazy_static;
use serde::Deserialize;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Display;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use tantivy::collector::{Count, DocSetCollector};
use tantivy::directory::MmapDirectory;
use tantivy::query::{AllQuery, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, Term, STORED, STRING, TEXT,
};
use tantivy::{DateTime, Searcher};
use tantivy::{DocAddress, Document, UserOperation};
use tantivy::{Index, IndexReader, IndexWriter, Opstamp};
use xdg::BaseDirectories;

#[cfg(feature = "graph")]
use dot::{GraphWalk, Labeller};
#[cfg(feature = "graph")]
use std::borrow::Cow;
use crate::Zest;

const TITLE_FIELD: &'static str = "title";
const CONTENT_FIELD: &'static str = "content";
const TAG_FIELD: &'static str = "tag";
const FILE_FIELD: &'static str = "file";
const PATH_FIELD: &'static str = "path";
const REF_FIELD: &'static str = "ref";
const LAST_MODIF_FIELD: &'static str = "lastmod";
const LANGUAGE: &'static str = "lang";
const CODE: &'static str = "code";

lazy_static! {
    static ref XDG_DIR: BaseDirectories =
        BaseDirectories::with_prefix("zest").expect("Impossible to create XDG directories");
}

#[derive(Deserialize, Default, Debug)]
struct Config {
    #[serde(default)]
    paths: Vec<String>,
}

struct DatabaseSchema {
    schema: Schema,
    title: Field,
    content: Field,
    tag: Field,
    file: Field,
    path: Field,
    lang: Field,
    code: Field,
    reff: Field,
    last_modif: Field,
}

impl DatabaseSchema {
    fn new() -> Self {
        let mut schema_builder = Schema::builder();
        let title = schema_builder.add_text_field(TITLE_FIELD, TEXT);
        let content = schema_builder.add_text_field(CONTENT_FIELD, TEXT);
        let tag = schema_builder.add_text_field(TAG_FIELD, STRING);
        let file = schema_builder.add_text_field(FILE_FIELD, TEXT);
        let path = schema_builder.add_text_field(PATH_FIELD, STRING | STORED);
        let reff = schema_builder.add_text_field(REF_FIELD, TEXT);
        let last_modif = schema_builder.add_date_field(LAST_MODIF_FIELD, STORED);
        let lang = schema_builder.add_text_field(LANGUAGE, TEXT);
        let code = schema_builder.add_text_field(CODE, TEXT);

        let schema = schema_builder.build();

        Self {
            schema,
            title,
            content,
            lang,
            code,
            tag,
            file,
            path,
            reff,
            last_modif,
        }
    }
}

#[derive(Debug)]
pub enum DatabaseError {
    ConfigError(String),
    DirectoryError(std::io::Error),
    OpenError(tantivy::directory::error::OpenDirectoryError),
    CreateError(tantivy::TantivyError),
    PutError(tantivy::TantivyError),
    QueryError(tantivy::query::QueryParserError),
    CorruptionError(&'static str),
}

impl Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DirectoryError(e) => e.fmt(f),
            Self::OpenError(e) => e.fmt(f),
            Self::CreateError(e) | Self::PutError(e) => e.fmt(f),
            Self::QueryError(e) => e.fmt(f),
            Self::ConfigError(e) => write!(f, "Configuration error: {}", e),
            Self::CorruptionError(e) => write!(f, "Corruption detected: {}", e),
        }
    }
}

impl Error for DatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DirectoryError(e) => Some(e),
            Self::OpenError(e) => Some(e),
            Self::CreateError(e) | Self::PutError(e) => Some(e),
            Self::QueryError(e) => Some(e),
            _ => None,
        }
    }
}

pub struct Database {
    config: Config,
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
}

impl Database {
    pub fn open() -> Result<Self, DatabaseError> {
        log::trace!("Open XDG directory");
        let dir = XDG_DIR
            .create_cache_directory("index")
            .map_err(|e| DatabaseError::DirectoryError(e))?;

        log::trace!("Open index");
        let dir = MmapDirectory::open(dir).map_err(|e| DatabaseError::OpenError(e))?;
        let index = Index::open_or_create(dir, DatabaseSchema::new().schema)
            .map_err(|e| DatabaseError::CreateError(e))?;

        log::trace!("Create writer and reader");
        let writer = index
            .writer(50_000_000)
            .map_err(|e| DatabaseError::CreateError(e))?;
        let reader = index.reader().map_err(|e| DatabaseError::CreateError(e))?;

        log::debug!("Open configuration");
        let conffile = XDG_DIR
            .place_config_file("config.yml")
            .map_err(|e| DatabaseError::DirectoryError(e))?;
        let config = if let Ok(conffile) = File::open(conffile) {
            let conffile = BufReader::new(conffile);
            if let Ok(c) = serde_yaml::from_reader(conffile) {
                c
            } else {
                Config::default()
            }
        } else {
            Config::default()
        };

        log::debug!("Using config : {:?}", config);

        Ok(Database {
            config,
            index,
            writer,
            reader,
        })
    }

    fn put_doc(&mut self, z: Zest, schema: &DatabaseSchema) {
        log::debug!("Inserting {:?}", z);
        let fname = std::fs::canonicalize(z.file).unwrap();
        let fname = fname.to_str().unwrap();

        log::trace!("Remove previously existing entries");
        self.writer
            .delete_term(Term::from_field_text(schema.path, fname));

        let metadata = std::fs::metadata(&fname).unwrap();
        let mut doc = Document::new();

        if let Ok(time) = metadata.modified() {
            let time = DateTime::from(time);
            log::trace!("Creating {} with modified time of {}", fname, time);
            doc.add_date(schema.last_modif, &time);
        } else {
            log::warn!("Could not retrieve {} last modified date.", fname);
        }
        doc.add_text(schema.title, z.title);
        doc.add_text(schema.file, fname.to_owned());
        doc.add_text(schema.path, fname.to_owned());
        doc.add_text(schema.content, z.content);

        for tag in z.metadata.tags {
            doc.add_text(schema.tag, tag);
        }

        for codeblock in z.codeblocks.iter() {

            if codeblock.code.is_some() {
                doc.add_text(schema.code, codeblock.code.as_ref().unwrap());
            }
            if codeblock.language.is_some() {
                doc.add_text(schema.lang, codeblock.language.as_ref().unwrap());
            }
        }

        for reff in z.refs {
            for matching in self.list(format!("file:{}", reff)).unwrap() {
                log::info!("{} references {}", fname, matching);
                doc.add_text(schema.reff, matching);
            }
        }

        log::debug!("Adding {:?}", doc);
        self.writer.add_document(doc);
    }

    fn commit(&mut self) -> Result<Opstamp, DatabaseError> {
        let op = self
            .writer
            .commit()
            .map_err(|e| DatabaseError::PutError(e))?;
        match self.reader.reload() {
            Ok(_) => Ok(op),
            Err(e) => Err(DatabaseError::PutError(e)),
        }
    }

    pub fn put(&mut self, z: Zest) -> Result<Opstamp, DatabaseError> {
        let schema = DatabaseSchema::new();
        self.put_doc(z, &schema);
        self.commit()
    }

    pub fn put_multiple(&mut self, zs: Vec<Zest>) -> Result<Opstamp, DatabaseError> {
        let schema = DatabaseSchema::new();
        for z in zs {
            self.put_doc(z, &schema);
        }
        self.commit()
    }

    pub fn search(&self, query: String) -> Result<Vec<Zest>, DatabaseError> {
        log::debug!("Searching with query: {}", query);
        let schema = DatabaseSchema::new();
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![schema.content, schema.title]);
        let q = query_parser
            .parse_query(query.as_ref())
            .map_err(|e| DatabaseError::QueryError(e))?;

        let docs: HashSet<DocAddress> = searcher.search(&q, &DocSetCollector).unwrap();

        let mut returned: Vec<Zest> = Vec::with_capacity(docs.len());
        for doc_address in docs {
            let doc = searcher.doc(doc_address).unwrap();
            let fname = doc
                .get_first(schema.path)
                .ok_or(DatabaseError::CorruptionError("missing path field"))?
                .text()
                .ok_or(DatabaseError::CorruptionError("wrong type for path field"))?
                .to_string();
            if let Ok(z) = Zest::from_file(fname) {
                returned.push(z);
            }
        }

        Ok(returned)
    }

    pub fn remove(&mut self, query: String) -> Result<Opstamp, DatabaseError> {
        log::debug!("Removing with query: {}", query);
        let schema = DatabaseSchema::new();
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![schema.content, schema.title]);

        log::trace!("Parse query");
        let q = query_parser
            .parse_query(query.as_ref())
            .map_err(|e| DatabaseError::QueryError(e))?;

        let to_execute = searcher
            .search(&q, &DocSetCollector)
            .unwrap()
            .iter()
            .filter_map(|doc_address| {
                let doc = searcher.doc(*doc_address).unwrap();
                let fname_field = if let Some(f) = doc.get_first(schema.path) {
                    f
                } else {
                    log::debug!("{:?} has no path field ?", doc_address);
                    return None;
                };

                let fname = if let Some(f) = fname_field.text() {
                    f
                } else {
                    log::debug!("{:?} path field is not of the correct type ?", doc_address);
                    return None;
                };

                Some(UserOperation::Delete(Term::from_field_text(
                    schema.path,
                    fname,
                )))
            })
            .collect();
        self.writer.run(to_execute);
        self.commit()
    }

    fn check_new(&mut self, schema: &DatabaseSchema, searcher: &Searcher) {
        // We're forced to do so because of the immutable borrow in the first for loop
        let mut new_docs: Vec<Zest> = Vec::new();
        for path in &self.config.paths {
            log::trace!("Looking into {}", path);
            if let Ok(dmeta) = std::fs::metadata(path) {
                if !dmeta.is_dir() {
                    log::warn!("{} is not a directory.", path);
                    continue;
                }

                for entry in walkdir::WalkDir::new(std::fs::canonicalize(path).unwrap())
                    .into_iter()
                    .filter_entry(|e| {
                        log::trace!("Considering {}", e.path().display());
                        e.file_name()
                            .to_str()
                            .map(|s| !s.starts_with("."))
                            .unwrap_or(false)
                    })
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                {
                    let entry = std::fs::canonicalize(entry.path()).unwrap();
                    let entry = entry.to_str().unwrap();
                    log::trace!("Checking {}", entry);
                    let query = TermQuery::new(
                        Term::from_field_text(schema.path, entry),
                        IndexRecordOption::Basic,
                    );

                    if searcher.search(&query, &Count).unwrap() == 0 {
                        // This file is not tracked yet, track it then
                        log::info!("{} is not tracked yet, adding it", entry);
                        if let Ok(z) = Zest::from_file(entry.to_owned()) {
                            new_docs.push(z);
                        } else {
                            log::warn!("Could not parse {}", entry);
                        }
                    }
                }
            }
        }
        for z in new_docs {
            self.put_doc(z, &schema);
        }
    }

    pub fn new(&mut self) -> Result<Opstamp, DatabaseError> {
        log::debug!("New start");
        let schema = DatabaseSchema::new();
        let searcher = self.reader.searcher();
        self.check_new(&schema, &searcher);
        self.commit()
    }

    pub fn update(&mut self) -> Result<Opstamp, DatabaseError> {
        log::debug!("Update start");
        let schema = DatabaseSchema::new();
        let searcher = self.reader.searcher();
        self.check_new(&schema, &searcher);
        for doc_address in searcher.search(&AllQuery, &DocSetCollector).unwrap() {
            let doc = searcher.doc(doc_address).unwrap();
            let fname = doc
                .get_first(schema.path)
                .ok_or(DatabaseError::CorruptionError("missing path field"))?
                .text()
                .ok_or(DatabaseError::CorruptionError("wrong type for path field"))?
                .to_string();
            let changetime = doc
                .get_first(schema.last_modif)
                .ok_or(DatabaseError::CorruptionError("missing file last_modified"))?
                .date_value()
                .ok_or(DatabaseError::CorruptionError(
                    "wrong type for last_modif field",
                ))?;

            if let Ok(meta) = std::fs::metadata(&fname) {
                let curr_changetime = DateTime::from(meta.modified().unwrap());
                if curr_changetime.timestamp() > changetime.timestamp() {
                    match Zest::from_file(fname.clone()) {
                        Ok(z) => {
                            log::debug!(
                                "{} has changed: {} > {}",
                                fname,
                                curr_changetime,
                                changetime
                            );
                            self.put_doc(z, &schema);
                        }
                        Err(e) => log::warn!("Could not update {}: {}", fname, e),
                    }
                } else {
                    log::trace!("No change detected for {}", fname);
                }
            } else {
                // Could not retrieve it, it must have been deleted
                self.writer
                    .delete_term(Term::from_field_text(schema.path, fname.as_ref()));
            }
        }

        self.commit()
    }

    /// Creates a new file, adds it to the database, and returns it's full path
    pub fn create(&mut self) -> Result<(String, Opstamp), DatabaseError> {
        if self.config.paths.is_empty() {
            return Err(DatabaseError::ConfigError(String::from(
                "The config does not specify paths",
            )));
        }
        let curtime = DateTime::from(std::time::SystemTime::now());
        let root = std::fs::canonicalize(self.config.paths.get(0).unwrap()).unwrap();
        let mut p = PathBuf::from(root);
        p.push(curtime.format("%Y_%m_%d_%H_%M_%S.md").to_string());

        let p = p.to_str().unwrap();
        File::create(p).unwrap();
        let z = if let Ok(z) = Zest::from_file(p.to_owned()) {
            z
        } else {
            unreachable!("zest should consider empty files as valid")
        };

        let opstamp = self.put(z)?;
        Ok((p.to_owned(), opstamp))
    }

    pub fn list(&mut self, query: String) -> Result<Vec<String>, DatabaseError> {
        log::debug!("Listing with query: {}", query);
        let schema = DatabaseSchema::new();
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![schema.content, schema.title]);
        let q = query_parser
            .parse_query(query.as_ref())
            .map_err(|e| DatabaseError::QueryError(e))?;

        let docs: HashSet<DocAddress> = searcher.search(&q, &DocSetCollector).unwrap();

        let mut returned: Vec<String> = Vec::with_capacity(docs.len());
        for doc_address in docs {
            let doc = searcher.doc(doc_address).unwrap();
            let fname = doc
                .get_first(schema.path)
                .ok_or(DatabaseError::CorruptionError("missing path field"))?
                .text()
                .ok_or(DatabaseError::CorruptionError("wrong type for path field"))?
                .to_string();
            returned.push(fname);
        }

        Ok(returned)
    }

    pub fn reindex(&mut self) -> Result<Opstamp, DatabaseError> {
        let tracked: Vec<Zest> = self.search(String::from("*"))?;
        self.writer
            .delete_all_documents()
            .map_err(|e| DatabaseError::PutError(e))?;
        self.put_multiple(tracked)
    }
}

#[cfg(feature = "graph")]
impl<'a> Labeller<'a, Zest, (Zest, Zest)> for Database {
    fn graph_id(&'a self) -> dot::Id<'a> {
        dot::Id::new("database").unwrap()
    }

    fn node_id(&'a self, n: &Zest) -> dot::Id<'a> {
        let meta = std::fs::metadata(n.file.clone()).unwrap();
        let mod_time = DateTime::from(meta.modified().unwrap());
        dot::Id::new(mod_time.format("N%Y%m%d%H%M%S").to_string()).unwrap()
    }

    fn node_label(&'a self, n: &Zest) -> dot::LabelText<'a> {
        dot::LabelText::label(n.title.clone())
    }
}

#[cfg(feature = "graph")]
impl<'a> GraphWalk<'a, Zest, (Zest, Zest)> for Database {
    fn nodes(&'a self) -> dot::Nodes<'a, Zest> {
        Cow::Owned(self.search(String::from("*")).unwrap())
    }

    fn edges(&'a self) -> dot::Edges<'a, (Zest, Zest)> {
        let nodes = self.search(String::from("*")).unwrap();

        // Not sure about this approximation, maybewe overapproximate, but this should avoid a lot
        // of allocations down the line
        let mut edges = Vec::with_capacity(nodes.len());
        for source in nodes {
            for dest in &source.refs {
                let matching_dests = self.search(format!("file:{}", dest)).unwrap();
                match matching_dests.len() {
                    0 => log::warn!("{} contains a broken link: {}", source.file, dest),
                    1 => {
                        edges.push((source.clone(), matching_dests.get(0).unwrap().clone()));
                    }
                    _ => {
                        log::warn!(
                            "{} contains a link that matches multiple files: {}",
                            source.file,
                            dest
                        );
                        for d in matching_dests {
                            edges.push((source.clone(), d));
                        }
                    }
                }
            }
        }

        return Cow::Owned(edges);
    }

    fn source(&'a self, edge: &(Zest, Zest)) -> Zest {
        edge.0.clone()
    }

    fn target(&'a self, edge: &(Zest, Zest)) -> Zest {
        edge.1.clone()
    }
}
