mod db;
mod zest;

#[macro_use]
extern crate clap;
use db::Database;
use log::error;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use zest::Zest;

fn main() {
    // let mut schema_builder = Schema::builder();
    // let title = schema_builder.add_text_field("title", TEXT);
    // let content = schema_builder.add_text_field("content", TEXT);
    // let tag = schema_builder.add_text_field("tag", TEXT);
    // let schema = schema_builder.build();
    // let doc = doc!(
    //     title => "foo",
    //     content => "bar",
    //     tag => "baz",
    //     tag => "bleh"
    //     );

    // println!("{:?}", doc);
    let matches = clap_app!(zest =>
      (author: "Thomas Vigouroux <tomvig38@gmail.com>")
      (@arg verbose: -v ... "Verbosity level")
      (@subcommand add =>
       (about: "Add documents to the database")
       (@arg FILE: +required ... "Files to add in the database")
      )
      (@subcommand search =>
       (about: "Search into the database for files")
       (@arg QUERY_TERMS: +required ... "Tantivy query to run") // We will actually concatenate those
      )
      (@subcommand remove =>
       (about: "Remove files matching the search term")
       (@arg QUERY_TERMS: +required ... "Tantivy query to run")
      )
      (@subcommand update =>
       (about: "Update all the files in the database")
      )
    )
    .get_matches();

    SimpleLogger::new()
        .with_level(match matches.occurrences_of("verbose") {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        })
        .with_colors(true)
        .init()
        .unwrap();

    let mut db = Database::new().unwrap();
    if let Some(matches) = matches.subcommand_matches("add") {
        for file in matches.values_of("FILE").unwrap() {
            match Zest::from_file(file.to_owned()) {
                Ok(z) => { db.put(z).unwrap(); },
                Err(e) => error!("{} is could not be successfully added: {}", file, e),
            }
        }
    } else if let Some(matches) = matches.subcommand_matches("search") {
        let terms: Vec<&str> = matches.values_of("QUERY_TERMS").unwrap().collect();
        let query = terms.join(" ");

        for r in db.search(query).unwrap() {
            println!("{}: {}", r.file, r.title);
        }
    } else if let Some(matches) = matches.subcommand_matches("remove") {
        let terms: Vec<&str> = matches.values_of("QUERY_TERMS").unwrap().collect();
        let query = terms.join(" ");
        db.remove(query).unwrap();
    } else if let Some(_) = matches.subcommand_matches("update") {
        db.update().unwrap();
    }
}