use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::process::Command;

fn main() {
    if let Err(e) = gen_reports() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn gen_reports() -> Result<(), Box<dyn Error>> {
    gen_balance()?;
    gen_expenses()?;
    gen_register()?;
    Ok(())
}

fn gen_balance() -> Result<(), Box<dyn Error>> {
    let balance_total = ledger("balance")?;
    let csv_output = ledger("csv assets")?;

    // Accumulate balance per flat account.
    let mut account_balances: BTreeMap<String, f64> = BTreeMap::new();
    for result in ledger_csv_reader(&csv_output).records() {
        let record = result?;
        let account = record[3].to_string();
        let amount: f64 = record[5].parse()?;
        *account_balances.entry(account).or_insert(0.0) += amount;
    }

    // Top-level categories (e.g. "travel" from "assets:travel").
    let categories: BTreeSet<String> = account_balances
        .keys()
        .filter_map(|acc| {
            let rest = acc.strip_prefix("assets:")?;
            Some(rest.split(':').next()?.to_string())
        })
        .collect();

    // For each category, compute balance, committed, and available.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut total_balance = 0.0f64;
    let mut total_committed = 0.0f64;
    for category in &categories {
        let prefix = format!("assets:{category}");
        let balance: f64 = account_balances
            .iter()
            .filter(|(acc, _)| *acc == &prefix || acc.starts_with(&format!("{prefix}:")))
            .map(|(_, &v)| v)
            .sum();
        if balance == 0.0 {
            continue;
        }
        let committed = account_balances
            .get(&format!("{prefix}:committed"))
            .copied()
            .unwrap_or(0.0);
        total_balance += balance;
        total_committed += committed;
        rows.push(vec![
            category.clone(),
            format_dollars(balance),
            if committed != 0.0 {
                format_dollars(committed)
            } else {
                String::new()
            },
            format_dollars(balance - committed),
        ]);
    }
    rows.push(vec![
        "**Total**".to_string(),
        format_dollars(total_balance),
        format_dollars(total_committed),
        format_dollars(total_balance - total_committed),
    ]);

    let table = markdown_table(&["Category", "Balance", "Committed", "Available"], &rows);
    let contents = format!(
        "# Balance

## Total

```
{balance_total}
```

## Budget position

{table}
"
    );
    fs::write("balance.md", contents)?;
    Ok(())
}

fn gen_expenses() -> Result<(), Box<dyn Error>> {
    let csv_output = ledger("csv expenses")?;

    // Aggregate amounts by category and year.
    let mut data: BTreeMap<String, BTreeMap<u32, f64>> = BTreeMap::new();
    for result in ledger_csv_reader(&csv_output).records() {
        let record = result?;
        let date = &record[0]; // e.g. "2024/03/26"
        let account = &record[3]; // e.g. "expenses:travel"
        let amount: f64 = record[5].parse()?;
        let year: u32 = date[..4].parse()?;
        let category = account
            .strip_prefix("expenses:")
            .unwrap_or(account)
            .to_string();
        *data.entry(category).or_default().entry(year).or_insert(0.0) += amount;
    }

    let year_list: Vec<u32> = data
        .values()
        .flat_map(|m| m.keys().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    // Build header and data rows.
    let year_strings: Vec<String> = year_list.iter().map(|y| y.to_string()).collect();
    let mut header: Vec<&str> = vec!["Category"];
    header.extend(year_strings.iter().map(|s| s.as_str()));
    header.push("Total");

    let mut rows: Vec<Vec<String>> = data
        .iter()
        .map(|(category, year_map)| {
            let mut row = vec![category.clone()];
            for year in &year_list {
                match year_map.get(year) {
                    Some(&amount) => row.push(format_dollars(amount)),
                    None => row.push(String::new()),
                }
            }
            row.push(format_dollars(year_map.values().sum()));
            row
        })
        .collect();

    // Totals row: sum each year column, then grand total.
    let mut totals_row = vec!["**Total**".to_string()];
    let mut grand_total = 0.0f64;
    for year in &year_list {
        let year_sum: f64 = data.values().filter_map(|m| m.get(year)).sum();
        grand_total += year_sum;
        totals_row.push(format_dollars(year_sum));
    }
    totals_row.push(format_dollars(grand_total));
    rows.push(totals_row);

    let table = markdown_table(&header, &rows);
    let contents = format!(
        "# Expenses

{table}"
    );
    fs::write("expenses.md", contents)?;
    Ok(())
}

fn gen_register() -> Result<(), Box<dyn Error>> {
    let stdout = ledger("register --monthly Assets --date-format %Y-%m-%d --columns=90")?;
    let contents = format!(
        "# Register

```
{stdout}
```
"
    );
    fs::write("register.md", contents)?;
    Ok(())
}

/// Render a GitHub Markdown table with aligned, fixed-width columns.
/// Column 0 is left-aligned; all other columns are right-aligned.
/// Include any totals row as the last entry in `rows`.
fn markdown_table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    let fmt_row = |cells: &[String]| -> String {
        let mut s = String::from("|");
        for (i, cell) in cells.iter().enumerate() {
            if i == 0 {
                s.push_str(&format!(" {:<w$} |", cell, w = widths[i]));
            } else {
                s.push_str(&format!(" {:>w$} |", cell, w = widths[i]));
            }
        }
        s.push('\n');
        s
    };

    let mut table = String::new();
    let header_row: Vec<String> = header.iter().map(|s| s.to_string()).collect();
    table.push_str(&fmt_row(&header_row));
    table.push('|');
    for (i, &w) in widths.iter().enumerate() {
        if i == 0 {
            table.push_str(&format!(" :{} |", "-".repeat(w - 1)));
        } else {
            table.push_str(&format!(" {}: |", "-".repeat(w - 1)));
        }
    }
    table.push('\n');
    for row in rows {
        table.push_str(&fmt_row(row));
    }
    table
}

fn ledger_csv_reader(csv_output: &str) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_output.as_bytes())
}

fn format_dollars(amount: f64) -> String {
    let cents = (amount * 100.0).round() as u64;
    let dollars = cents / 100;
    let frac = cents % 100;
    let s = dollars.to_string();
    let mut with_commas = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            with_commas.push(',');
        }
        with_commas.push(ch);
    }
    let dollars_str: String = with_commas.chars().rev().collect();
    format!("${dollars_str}.{frac:02}")
}

fn ledger(args: &str) -> Result<String, Box<dyn Error>> {
    let output = Command::new("ledger")
        .args(["-f", "budget.dat"])
        .args(args.split_whitespace().collect::<Vec<_>>())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap();
        return Err(format!("ledger failed:\n{stderr}").into());
    }
    let stdout = String::from_utf8(output.stdout).unwrap();
    Ok(stdout)
}
