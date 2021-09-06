use lazy_static::lazy_static;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Display;
use tantivy::collector::DocSetCollector;
use tantivy::directory::MmapDirectory;
use tantivy::query::{AllQuery, QueryParser};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, Term, TextFieldIndexing, TextOptions, STORED, STRING, TEXT,
};
use tantivy::tokenizer::*;
use tantivy::{DocAddress, Document, Score, UserOperation};
use tantivy::{Index, IndexReader, IndexWriter, Opstamp};
use xdg::{BaseDirectories, BaseDirectoriesError};

use crate::Zest;

const TITLE_FIELD: &'static str = "title";
const CONTENT_FIELD: &'static str = "content";
const TAG_FIELD: &'static str = "tag";
const FILE_FIELD: &'static str = "file";
const REF_FIELD: &'static str = "ref";
const LAST_MODIF_FIELD: &'static str = "ref";
const CUSTOM_TOKENIZER: &'static str = "custom";

lazy_static! {
    static ref XDG_DIR: BaseDirectories = BaseDirectories::with_prefix(clap::crate_name!())
        .expect("Impossible to create XDG directories");
}

struct DatabaseSchema {
    schema: Schema,
    title: Field,
    content: Field,
    tag: Field,
    file: Field,
    reff: Field,
    last_modif: Field,
}

impl DatabaseSchema {
    fn new() -> Self {
        fn make_text_options() -> TextOptions {
            let text_indexing = TextFieldIndexing::default()
                .set_tokenizer(CUSTOM_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions);
            TextOptions::default().set_indexing_options(text_indexing)
        }

        let mut schema_builder = Schema::builder();
        let title = schema_builder.add_text_field(TITLE_FIELD, make_text_options());
        let content = schema_builder.add_text_field(CONTENT_FIELD, make_text_options());
        let tag = schema_builder.add_text_field(TAG_FIELD, STRING);
        let file = schema_builder.add_text_field(FILE_FIELD, TEXT | STORED);
        let reff = schema_builder.add_text_field(REF_FIELD, TEXT);
        let last_modif = schema_builder.add_date_field(LAST_MODIF_FIELD, STORED);

        let schema = schema_builder.build();

        Self {
            schema,
            title,
            content,
            tag,
            file,
            reff,
            last_modif,
        }
    }
}

#[derive(Debug)]
pub enum DatabaseError {
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
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
}

impl Database {
    pub fn new() -> Result<Self, DatabaseError> {
        log::trace!("Open XDG directory");
        let dir = XDG_DIR
            .create_cache_directory("index")
            .map_err(|e| DatabaseError::DirectoryError(e))?;

        log::trace!("Creating tokenizers");
        let custom_stem = TextAnalyzer::from(SimpleTokenizer)
            .filter(RemoveLongFilter::limit(40))
            .filter(LowerCaser)
            .filter(Stemmer::new(Language::English))
            .filter(Stemmer::new(Language::French));

        log::trace!("Open index");
        let dir = MmapDirectory::open(dir).map_err(|e| DatabaseError::OpenError(e))?;
        let index = Index::open_or_create(dir, DatabaseSchema::new().schema)
            .map_err(|e| DatabaseError::CreateError(e))?;

        index.tokenizers().register(CUSTOM_TOKENIZER, custom_stem);

        log::trace!("Create writer and reader");
        let writer = index
            .writer(50_000_000)
            .map_err(|e| DatabaseError::CreateError(e))?;
        let reader = index.reader().map_err(|e| DatabaseError::CreateError(e))?;
        Ok(Database {
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
        self.writer.delete_term(Term::from_field_text(schema.file, fname));

        let metadata = std::fs::metadata(&fname).unwrap();
        let mut doc = Document::new();

        if let Ok(time) = metadata.modified() {
            doc.add_date(schema.last_modif, &tantivy::DateTime::from(time));
        } else {
            log::warn!("Could not retrieve {} last modified date.", fname);
        }
        doc.add_text(schema.title, z.title);
        doc.add_text(schema.file, fname.to_owned());
        doc.add_text(schema.content, z.content);

        for tag in z.metadata.tags {
            doc.add_text(schema.tag, tag);
        }

        for reff in z.refs {
            doc.add_text(schema.reff, reff);
        }

        self.writer.add_document(doc);
    }

    pub fn put(&mut self, z: Zest) -> Result<Opstamp, DatabaseError> {
        let schema = DatabaseSchema::new();
        self.put_doc(z, &schema);
        self.writer.commit().map_err(|e| DatabaseError::PutError(e))
    }

    pub fn put_multiple(&mut self, zs: Vec<Zest>) -> Result<Opstamp, DatabaseError> {
        let schema = DatabaseSchema::new();
        for z in zs {
            self.put_doc(z, &schema);
        }
        self.writer.commit().map_err(|e| DatabaseError::PutError(e))
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
                .get_first(schema.file)
                .ok_or(DatabaseError::CorruptionError("missing file field"))?
                .text()
                .ok_or(DatabaseError::CorruptionError("wrong type for file field"))?
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
                let fname_field = if let Some(f) = doc.get_first(schema.file) {
                    f
                } else {
                    log::debug!("{:?} has no file field ?", doc_address);
                    return None;
                };

                let fname = if let Some(f) = fname_field.text() {
                    f
                } else {
                    log::debug!("{:?} file field is not of the correct type ?", doc_address);
                    return None;
                };

                Some(UserOperation::Delete(Term::from_field_text(
                    schema.file,
                    fname,
                )))
            })
            .collect();
        self.writer.run(to_execute);
        self.writer.commit().map_err(|e| DatabaseError::PutError(e))
    }

    pub fn update(&mut self) -> Result<Opstamp, DatabaseError> {
        let schema = DatabaseSchema::new();
        let searcher = self.reader.searcher();
        for doc_address in searcher.search(&AllQuery, &DocSetCollector).unwrap() {
            let doc = searcher.doc(doc_address).unwrap();
            let fname = doc
                .get_first(schema.file)
                .ok_or(DatabaseError::CorruptionError("missing file field"))?
                .text()
                .ok_or(DatabaseError::CorruptionError("wrong type for file field"))?
                .to_string();
            let changetime = doc
                .get_first(schema.last_modif)
                .ok_or(DatabaseError::CorruptionError("missing file last_modified"))?
                .date_value()
                .ok_or(DatabaseError::CorruptionError("wrong type for last_modif field"))?;

            if let Ok(meta) = std::fs::metadata(&fname) {
                let curr_changetime = tantivy::DateTime::from(meta.modified().unwrap());
                if  curr_changetime.timestamp() > changetime.timestamp() {
                    match Zest::from_file(fname.clone()) {
                        Ok(z) => {
                            log::debug!("{} has changed: {} > {}", fname, curr_changetime, changetime);
                            self.put_doc(z, &schema);
                        },
                        Err(e) => log::warn!("Could not update {}: {}", fname, e)
                    }
                }
            } else {
                // Could not retrieve it, it must have been deleted
                self.writer.delete_term(Term::from_field_text(schema.file, fname.as_ref()));
            }
        }

        self.writer.commit().map_err(|e| DatabaseError::PutError(e))
    }
}
