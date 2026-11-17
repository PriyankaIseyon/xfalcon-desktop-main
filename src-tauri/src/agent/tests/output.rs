use xfalcon_contracts::credentials::ModelId;
use xfalcon_contracts::RunId;

use crate::agent::output::{FinalAssistantOutput, OutputError, RunOutputBuilder};

#[test]
fn builds_bounded_text_output() {
    let mut builder = RunOutputBuilder::new(RunId::from("run-1"), ModelId::from("gpt"), 32);
    builder.push_text("hello").expect("fits");
    let built = builder.finish().expect("finish");
    assert_eq!(built.text(), "hello");
    assert!(!built.had_tool_request());
}

#[test]
fn rejects_oversized_chunk() {
    let mut builder = RunOutputBuilder::new(RunId::from("run-1"), ModelId::from("gpt"), 4);
    let err = builder.push_text("hello").expect_err("too large");
    assert_eq!(err, OutputError::ByteBudgetExceeded);
}

#[test]
fn rejects_empty_finish() {
    let builder = RunOutputBuilder::new(RunId::from("run-1"), ModelId::from("gpt"), 64);
    let err = builder.finish().expect_err("empty");
    assert_eq!(err, OutputError::EmptyOutput);
}

#[test]
fn tool_request_surfaces_as_unavailable() {
    let mut builder = RunOutputBuilder::new(RunId::from("run-1"), ModelId::from("gpt"), 64);
    builder.note_tool_request();
    builder.push_text("partial").expect("text");
    let built = builder.finish().expect("finish");
    assert!(built.had_tool_request());
    assert_eq!(built.text(), "partial");
}

#[test]
fn final_output_validates_artifact_refs_are_ids_only() {
    let mut builder = RunOutputBuilder::new(RunId::from("run-1"), ModelId::from("gpt"), 64);
    builder.push_text("done").expect("text");
    let built = builder.finish().expect("finish");
    let final_out = FinalAssistantOutput::from_builder(built).expect("valid");
    assert_eq!(final_out.content(), "done");
    assert!(final_out.artifact_refs().is_empty());
}
