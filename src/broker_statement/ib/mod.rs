use std::collections::HashMap;
use std::iter::Iterator;
#[cfg(test)] use std::path::Path;

use csv::{self, StringRecord};

use core::GenericResult;
use currency::Cash;
use broker_statement::{BrokerStatement, BrokerStatementBuilder};
use broker_statement::ib::common::{Record, RecordParser, format_record};

mod common;
mod dividends;
mod parsers;
mod taxes;

enum State {
    None,
    Record(StringRecord),
    Header(StringRecord),
}

pub struct IbStatementParser {
    statement: BrokerStatementBuilder,
    tickers: HashMap<String, String>,
    taxes: HashMap<taxes::TaxId, Cash>,
    dividends: Vec<dividends::DividendInfo>,
}

impl IbStatementParser {
    pub fn new() -> IbStatementParser {
        IbStatementParser {
            statement: BrokerStatementBuilder::new(),
            tickers: HashMap::new(),
            taxes: HashMap::new(),
            dividends: Vec::new(),
        }
    }

    pub fn parse(mut self, path: &str) -> GenericResult<BrokerStatement> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_path(path)?;

        let mut records = reader.records();
        let mut state = Some(State::None);

        'state: loop {
            match state.take().unwrap() {
                State::None => {
                    match records.next() {
                        Some(result) => state = Some(State::Record(result?)),
                        None => break,
                    };
                }
                State::Record(record) => {
                    if record.len() < 2 {
                        return Err!("Invalid record: {}", format_record(&record));
                    }

                    if record.get(1).unwrap() == "Header" {
                        state = Some(State::Header(record));
                    } else if record.get(1).unwrap() == "" {
                        trace!("Headerless record: {}.", format_record(&record));
                        state = Some(State::None);
                    } else {
                        return Err!("Invalid record: {}", format_record(&record));
                    }
                },
                State::Header(record) => {
                    let (name, fields) = parse_header(&record)?;

                    let parser: Box<RecordParser> = match name {
                        "Statement" => Box::new(parsers::StatementInfoParser {}),
                        "Net Asset Value" => Box::new(parsers::NetAssetValueParser {}),
                        "Deposits & Withdrawals" => Box::new(parsers::DepositsParser {}),
                        "Dividends" => Box::new(dividends::DividendsParser {}),
                        "Withholding Tax" => Box::new(taxes::WithholdingTaxParser {}),
                        "Financial Instrument Information" => Box::new(parsers::FinancialInstrumentInformationParser {}),
                        _ => Box::new(parsers::UnknownRecordParser {}),
                    };

                    let data_types = parser.data_types();

                    while let Some(result) = records.next() {
                        let record = result?;

                        if record.len() < 2 {
                            return Err!("Invalid record: {}", format_record(&record));
                        }

                        if record.get(0).unwrap() != name {
                            state = Some(State::Record(record));
                            continue 'state;
                        } else if record.get(1).unwrap() == "Header" {
                            state = Some(State::Header(record));
                            continue 'state;
                        }

                        if let Some(data_types) = data_types {
                            if !data_types.contains(&record.get(1).unwrap()) {
                                return Err!("Invalid data record type: {}", format_record(&record));
                            }
                        }

                        parser.parse(&mut self, &Record {
                            name: name,
                            fields: &fields,
                            values: &record,
                        }).map_err(|e| format!(
                            "Failed to parse ({}) record: {}", format_record(&record), e
                        ))?;
                    }

                    break;
                }
            }
        }

        self.statement.dividends = dividends::parse_dividends(self.dividends, &mut self.taxes)?;

        Ok(self.statement.get().map_err(|e| format!("Invalid statement: {}", e))?)
    }
}

fn parse_header(record: &StringRecord) -> GenericResult<(&str, Vec<&str>)> {
    let name = record.get(0).unwrap();
    let fields = record.iter().skip(2).collect::<Vec<_>>();
    trace!("Header: {}: {}.", name, format_record(fields.iter().map(|field: &&str| *field)));
    Ok((name, fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        let path = Path::new(file!()).parent().unwrap().join("testdata/statement.csv");
        let statement = IbStatementParser::new().parse(path.to_str().unwrap()).unwrap();

        assert!(statement.deposits.len() > 0);
        assert!(statement.dividends.len() > 0);
    }
}