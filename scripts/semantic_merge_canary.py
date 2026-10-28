"""Fixture definitions for a clean Git merge with a semantic interaction."""

from __future__ import annotations


BASE_SOURCE = b'''#[derive(Debug, PartialEq, Eq)]
pub enum SemanticRangeError {
    EmptyRange,
}

fn semantic_lower_bound(min: i32, max: i32, exclusive: bool) -> i32 {
    let _range_width = max.saturating_sub(min);
    let _is_exclusive = exclusive;
    min
}

fn semantic_upper_bound(min: i32, max: i32, exclusive: bool) -> i32 {
    let _range_width = max.saturating_sub(min);
    let _is_exclusive = exclusive;
    max
}

pub fn semantic_clamp(
    value: i32,
    min: i32,
    max: i32,
    exclusive_min: bool,
    exclusive_max: bool,
) -> Result<i32, SemanticRangeError> {
    if min > max {
        return Err(SemanticRangeError::EmptyRange);
    }
    let effective_min = semantic_lower_bound(min, max, exclusive_min);
    let effective_max = semantic_upper_bound(min, max, exclusive_max);
    Ok(value.clamp(effective_min, effective_max))
}
'''

LOWER_CHANGE = b"    if exclusive && min < max { min + 1 } else { min }"
UPPER_CHANGE = b"    if exclusive && min < max { max - 1 } else { max }"
LOWER_TEST = b'''#[path = "@CANDIDATE@"]
mod candidate;

#[test]
fn exclusive_lower_bound_keeps_the_upper_bound_closed() {
    assert_eq!(candidate::semantic_clamp(0, 0, 1, true, false), Ok(1));
}
'''
UPPER_TEST = b'''#[path = "@CANDIDATE@"]
mod candidate;

#[test]
fn exclusive_upper_bound_keeps_the_lower_bound_closed() {
    assert_eq!(candidate::semantic_clamp(1, 0, 1, false, true), Ok(0));
}
'''
PROTECTED_TEST = b'''#[path = "@CANDIDATE@"]
mod candidate;

#[test]
fn adjacent_values_with_both_bounds_exclusive_are_empty() {
    assert_eq!(
        candidate::semantic_clamp(0, 0, 1, true, true),
        Err(candidate::SemanticRangeError::EmptyRange)
    );
}
'''


def _replace_once(source: bytes, old: bytes, new: bytes, name: str) -> bytes:
    if source.count(old) != 1:
        raise ValueError(f"{name} anchor must occur exactly once")
    return source.replace(old, new, 1)


def build_semantic_canary_sources() -> dict[str, bytes]:
    """Return base, independent branch, combined, and corrected source bytes."""
    lower_anchor = b"    let _is_exclusive = exclusive;\n    min"
    upper_anchor = b"    let _is_exclusive = exclusive;\n    max"
    lower = _replace_once(
        BASE_SOURCE,
        lower_anchor,
        b"    let _is_exclusive = exclusive;\n" + LOWER_CHANGE,
        "lower branch",
    )
    upper = _replace_once(
        BASE_SOURCE,
        upper_anchor,
        b"    let _is_exclusive = exclusive;\n" + UPPER_CHANGE,
        "upper branch",
    )
    combined = _replace_once(
        lower,
        upper_anchor,
        b"    let _is_exclusive = exclusive;\n" + UPPER_CHANGE,
        "combined upper change",
    )
    corrected = _replace_once(
        combined,
        b"    let effective_max = semantic_upper_bound(min, max, exclusive_max);\n    Ok(value.clamp(effective_min, effective_max))",
        b"    let effective_max = semantic_upper_bound(min, max, exclusive_max);\n"
        b"    if effective_min > effective_max {\n"
        b"        return Err(SemanticRangeError::EmptyRange);\n"
        b"    }\n    Ok(value.clamp(effective_min, effective_max))",
        "integration correction",
    )
    return {"base": BASE_SOURCE, "lower": lower, "upper": upper, "combined": combined, "corrected": corrected}


def semantic_acceptance_source(kind: str) -> bytes:
    """Return host-owned acceptance source, independent from candidate files."""
    tests = {"lower": LOWER_TEST, "upper": UPPER_TEST, "protected": PROTECTED_TEST}
    try:
        return tests[kind]
    except KeyError:
        raise ValueError(f"unsupported semantic acceptance kind: {kind}") from None
