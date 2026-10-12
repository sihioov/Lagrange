#[path = "../../../../data-pipelines/collectors/src/calendar_claim.rs"]
mod calendar_claim;

use calendar_claim::CalendarDayLock;
use domain::{BatchId, TradingDate};
use std::path::Path;
use std::str::FromStr;

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().expect("command");
    let root = args.next().expect("absolute raw root");
    let date = TradingDate::parse(&args.next().expect("date")).expect("valid date");
    assert!(args.next().is_none(), "unexpected argument");

    match command.as_str() {
        "create" => {
            let guard = CalendarDayLock::acquire(Path::new(&root), date)
                .expect("initial CalendarDayLock acquire");
            let batch = BatchId::from_str("11111111-1111-4111-8111-111111111111")
                .expect("fixture batch UUID");
            guard.consume(batch).expect("consume claim");
            assert_eq!(guard.existing().expect("read claim"), Some(batch));
            println!("CREATE_OK");
        }
        "acquire" => match CalendarDayLock::acquire(Path::new(&root), date) {
            Ok(guard) => {
                assert_eq!(
                    guard.existing().expect("read claim"),
                    Some(BatchId::from_str("11111111-1111-4111-8111-111111111111").unwrap())
                );
                println!("ACQUIRE_OK");
            }
            Err(error) => {
                eprintln!("ACQUIRE_ERR {error}");
                std::process::exit(2);
            }
        },
        _ => panic!("unknown command"),
    }
}
