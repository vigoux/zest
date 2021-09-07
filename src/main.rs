mod db;
mod zest;

#[macro_use]
extern crate clap;
use db::Database;
use log::error;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use std::error::Error;
use zest::Zest;

fn main() -> Result<(), Box<dyn Error>> {
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
    let mut app = clap_app!(zest =>
      (author: "Thomas Vigouroux <tomvig38@gmail.com>")
      (@arg verbose: -v ... "Verbosity level")
      (@subcommand add =>
       (about: "Add documents to the database")
       (@arg FILE: +required ... "Files to add in the database")
      )
      (@subcommand search =>
       (about: "Search into the database for files and print their files and titles")
       (@arg only_files: -f --only-files "Only print file paths")
       (@arg QUERY_TERMS: +required ... "Tantivy query to run") // We will actually concatenate those
      )
      (@subcommand remove =>
       (about: "Remove files matching the search term")
       (@arg QUERY_TERMS: +required ... "Tantivy query to run")
      )
      (@subcommand update =>
       (about: "Synchronizes the database, also checks for new files")
      )
      (@subcommand new =>
       (about: "Checks for new files in the database")
       )
      (@subcommand create =>
       (about: "Creates a new file, add it to the database, and returns it's path")
       )
      (@subcommand reindex =>
       (about: "Reindexes the whole database as once. If some links are broken, this could fix it")
       )
    )
    .setting(clap::AppSettings::ArgRequiredElseHelp);

    #[cfg(feature = "graph")]
    {
        app = app.subcommand(
            clap::SubCommand::with_name("graph").about("Shows a graph representing the database"),
        );
    }

    let matches = app.get_matches();

    SimpleLogger::new()
        .with_level(match matches.occurrences_of("verbose") {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        })
        .with_colors(true)
        .init()?;

    let mut db = Database::open()?;

    if matches.subcommand_matches("update").is_some() {
        db.update()?;
        return Ok(());
    }

    if matches.subcommand_matches("new").is_some() {
        db.new()?;
        return Ok(());
    }

    if let Some(matches) = matches.subcommand_matches("search") {
        let terms: Vec<&str> = matches.values_of("QUERY_TERMS").unwrap().collect();
        let query = terms.join(" ");

        if matches.is_present("only_files") {
            for f in db.list(query)? {
                println!("{}", f);
            }
        } else {
            for r in db.search(query)? {
                println!("{}: {}", r.file, r.title);
            }
        }
        return Ok(());
    }

    if matches.subcommand_matches("create").is_some() {
        let (path, _) = db.create()?;
        println!("{}", path);
        return Ok(());
    }

    if matches.subcommand_matches("reindex").is_some() {
        db.reindex()?;
        return Ok(());
    }

    if let Some(matches) = matches.subcommand_matches("remove") {
        let terms: Vec<&str> = matches.values_of("QUERY_TERMS").unwrap().collect();
        let query = terms.join(" ");
        db.remove(query)?;
        return Ok(());
    }

    if let Some(matches) = matches.subcommand_matches("add") {
        let to_add: Vec<Zest> = matches
            .values_of("FILE")
            .unwrap()
            .filter_map(|fname| match Zest::from_file(fname.to_owned()) {
                Ok(z) => Some(z),
                Err(e) => {
                    error!("{} is could not be successfully added: {}", fname, e);
                    None
                }
            })
            .collect();
        db.put_multiple(to_add)?;
        return Ok(());
    }

    #[cfg(feature = "graph")]
    if matches.subcommand_matches("graph").is_some() {
        let mut tmp_dir = std::env::temp_dir();
        tmp_dir.push("graph.dot");
        let path = tmp_dir.to_str().unwrap();
        println!("{}", path);
        let mut file = std::fs::File::create(tmp_dir)?;
        dot::render(&db, &mut file).unwrap();
        return Ok(());
    }

    Ok(())
}
