use crate::pack::PackError;

const MAX_RESULTS: usize = 1000;
const MAX_QUERY_BYTES: usize = 4096;
const MAX_RECURSION_BUDGET: usize = 32;

/// Conservative spike limits before Tantivy's allocating collector/recursive parser.
pub(super) fn validate(query: &str, limit: usize) -> Result<(), PackError> {
    if limit > MAX_RESULTS {
        return Err(invalid("search limit exceeds 1,000 results"));
    }
    if query.len() > MAX_QUERY_BYTES {
        return Err(invalid("search query exceeds 4 KiB"));
    }
    let groups = query.bytes().filter(|byte| *byte == b'(').count();
    let nots = query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| *word == "NOT")
        .count();
    // A raw upper bound deliberately includes literal/escaped/regex/range text.
    // Reimplementing the parser's quote rules can hide actual recursive syntax.
    if groups + nots > MAX_RECURSION_BUDGET {
        return Err(invalid(
            "search query exceeds the textual group/NOT budget of 32",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> PackError {
    PackError::Mismatch(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textual_budget_cannot_be_hidden_by_regex_or_range_quotes() {
        for query in [
            format!("/\"/ AND {}knot OR /\"/", "NOT ".repeat(500)),
            format!(
                "title:['A TO Z] AND {}knot OR title:['A TO Z]",
                "NOT ".repeat(500)
            ),
            format!("\"{}\"", "(NOT) ".repeat(40)),
        ] {
            // Guard only: never submit these cases to the unbounded parser.
            assert!(matches!(validate(&query, 10), Err(PackError::Mismatch(_))));
        }
    }
}
