use super::*;

fn prepared() -> PreparedAnswer {
    PreparedAnswer {
        prompt: Some("checked prompt".into()),
        retrieval_us: 0,
        extraction_us: 0,
        sources: (1..=3)
            .map(|number| AnswerSource {
                number,
                title: format!("Title {number}"),
                url: format!("C/Article_{number}"),
                excerpt: format!("Evidence {number}"),
            })
            .collect(),
    }
}

#[test]
fn citations_must_reference_retrieved_sources() {
    let answer = finish_answer(
        &prepared(),
        "Claim [1] [4] [0] [01] [999999999999999999999]".into(),
        GenerationEnd::Eos,
    )
    .unwrap();
    assert_eq!(answer.citations, [1]);
    assert!(answer.flags.invalid_citations && !answer.flags.uncited);
    assert_eq!(answer.sources[0].url, "C/Article_1");
    let overflow = finish_answer(
        &prepared(),
        "Claim [999999999999999999999".into(),
        GenerationEnd::Eos,
    )
    .unwrap();
    assert!(overflow.flags.invalid_citations && overflow.flags.uncited);
}

#[test]
fn answer_without_any_citation_is_flagged_uncited() {
    let answer = finish_answer(
        &prepared(),
        "An unsupported answer.".into(),
        GenerationEnd::Eos,
    )
    .unwrap();
    assert!(answer.flags.uncited);
    assert!(answer.citations.is_empty());
}

#[test]
fn leading_thinking_is_excluded_from_visible_citation_authority() {
    let raw = "  <think>hidden [1]</think>\nVisible [2]";
    let answer = finish_answer(&prepared(), raw.into(), GenerationEnd::Eos).unwrap();
    assert_eq!(answer.raw_text, raw);
    assert_eq!(answer.text, "Visible [2]");
    assert_eq!(answer.citations, [2]);
    assert!(answer.flags.nonempty_thinking && !answer.flags.unfinished_thinking);
    let empty = finish_answer(
        &prepared(),
        "<think> \n</think> Answer [3]".into(),
        GenerationEnd::Eos,
    )
    .unwrap();
    assert!(!empty.flags.nonempty_thinking);
    let nested = finish_answer(
        &prepared(),
        "<think>outer <think>[1]</think> tail</think> Answer [2]".into(),
        GenerationEnd::Eos,
    )
    .unwrap();
    assert_eq!(nested.text, "Answer [2]");
    assert_eq!(nested.citations, [2]);
}

#[test]
fn unfinished_thinking_and_limit_are_flagged_without_invented_links() {
    let answer = finish_answer(
        &prepared(),
        "<think>still reasoning [1]".into(),
        GenerationEnd::Limit,
    )
    .unwrap();
    assert!(answer.text.is_empty() && answer.citations.is_empty());
    assert!(
        answer.flags.unfinished_thinking
            && answer.flags.nonempty_thinking
            && answer.flags.truncated
            && answer.flags.uncited
    );
    let cancelled =
        finish_answer(&prepared(), "Partial [1]".into(), GenerationEnd::Cancelled).unwrap();
    assert!(cancelled.flags.incomplete && !cancelled.flags.truncated);
}

#[test]
fn model_urls_never_replace_retrieved_source_urls() {
    let answer = finish_answer(
        &prepared(),
        "[1](https://example.invalid/fake)".into(),
        GenerationEnd::Eos,
    )
    .unwrap();
    assert_eq!(answer.citations, [1]);
    assert_eq!(answer.sources[0].url, "C/Article_1");
}

#[test]
fn excerpt_is_a_fixed_utf8_prefix_and_prompt_is_deterministic() {
    assert_eq!(
        excerpt(&format!("{}🌎tail", "a".repeat(1023))).unwrap(),
        "a".repeat(1023)
    );
    assert!(excerpt(" \n").is_err());
    let sources = prepared().sources;
    let first = numbered_prompt("Question?", &sources).unwrap().unwrap();
    assert_eq!(
        numbered_prompt("Question?", &sources).unwrap().unwrap(),
        first
    );
    assert!(first.ends_with("/no_think"));
    assert!(first.contains("[1] Title 1\nC/Article_1\nEvidence 1"));
    assert!(first.find("[1]").unwrap() < first.find("[2]").unwrap());
    assert!(numbered_prompt("Question?", &[]).unwrap().is_none());
    for question in ["".to_owned(), "x".repeat(4097), "bad\0question".into()] {
        assert!(matches!(
            numbered_prompt(&question, &sources),
            Err(PackError::Mismatch(_))
        ));
    }
    let mut invalid = sources;
    invalid[0].number = 4;
    assert!(numbered_prompt("Question?", &invalid).is_err());
}

#[test]
fn final_output_has_the_existing_generation_byte_bound() {
    assert!(matches!(
        finish_answer(&prepared(), "x".repeat(128 * 1024 + 1), GenerationEnd::Eos),
        Err(PackError::Mismatch(_))
    ));
}
