//! Actual public SDK parser/generator regressions, folded into the parser canary.
#![allow(clippy::unwrap_used)]
use grammers_client::parsers::*;
use grammers_tl_types::{enums::MessageEntity as E, types as t};
#[path = "../../vendor/grammers/grammers-client/src/utils/clock.rs"]
mod sdk_utils;
pub fn run() {
    let epoch = std::time::SystemTime::UNIX_EPOCH;
    assert_eq!(
        sdk_utils::random_id_seed(epoch - std::time::Duration::from_secs(1)),
        1
    );
    assert_eq!(sdk_utils::random_id_seed(epoch), 1);
    // Windows SystemTime uses 100 ns units; one microsecond is exactly representable.
    assert_eq!(
        sdk_utils::random_id_seed(epoch + std::time::Duration::from_micros(1)),
        1_000
    );
    let probes: &[(&str, fn())] = &[
        ("markdown_block_boundaries", || {
            for (input, expected) in [
                ("# Heading\n\nBody", "Heading\n\nBody"),
                ("Heading\n=======\n\nBody", "Heading\n\nBody"),
                ("Heading\n-------\n\nBody", "Heading\n\nBody"),
                ("- one\n- two", "one\n\ntwo"),
                ("> Quote\n\nBody", "Quote\n\nBody"),
                ("One\n\n---\n\nTwo", "One\n\nTwo"),
            ] {
                assert_eq!(parse_markdown_message(input), (expected.into(), vec![]));
            }
        }),
        ("markdown_emphasis_whitespace", || {
            for italic in [false, true] {
                let entity: E = if italic {
                    t::MessageEntityItalic {
                        offset: 0,
                        length: 9,
                    }
                    .into()
                } else {
                    t::MessageEntityBold {
                        offset: 0,
                        length: 9,
                    }
                    .into()
                };
                let expected: E = if italic {
                    t::MessageEntityItalic {
                        offset: 1,
                        length: 7,
                    }
                    .into()
                } else {
                    t::MessageEntityBold {
                        offset: 1,
                        length: 7,
                    }
                    .into()
                };
                let generated = generate_markdown_message(" hello😀 ", &[entity]);
                assert_eq!(
                    parse_markdown_message(&generated),
                    (" hello😀 ".into(), vec![expected])
                );
                let all_space: E = if italic {
                    t::MessageEntityItalic {
                        offset: 0,
                        length: 4,
                    }
                    .into()
                } else {
                    t::MessageEntityBold {
                        offset: 0,
                        length: 4,
                    }
                    .into()
                };
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message("    ", &[all_space])),
                    ("    ".into(), vec![])
                );
            }
        }),
        ("markdown_adjacent_code", || {
            for (text, first, second) in [("ab", 1, 1), ("😀b", 2, 1), ("a`b", 1, 2)] {
                let entities = [
                    t::MessageEntityCode {
                        offset: 0,
                        length: first,
                    }
                    .into(),
                    t::MessageEntityCode {
                        offset: first,
                        length: second,
                    }
                    .into(),
                ];
                let generated = generate_markdown_message(text, &entities);
                assert_eq!(
                    parse_markdown_message(&generated),
                    (
                        text.into(),
                        vec![
                            t::MessageEntityCode {
                                offset: 0,
                                length: first + second
                            }
                            .into()
                        ]
                    )
                );
            }
            let separate = [
                t::MessageEntityCode {
                    offset: 0,
                    length: 1,
                }
                .into(),
                t::MessageEntityCode {
                    offset: 2,
                    length: 1,
                }
                .into(),
            ];
            assert_eq!(
                parse_markdown_message(&generate_markdown_message("a b", &separate)),
                ("a b".into(), separate.to_vec())
            );
        }),
        ("markdown_adjacent_emphasis", || {
            for italic in [false, true] {
                let entity = |offset, length| -> E {
                    if italic {
                        t::MessageEntityItalic { offset, length }.into()
                    } else {
                        t::MessageEntityBold { offset, length }.into()
                    }
                };
                for (text, spans) in [
                    ("ab", vec![(0, 1), (1, 1)]),
                    ("😀b", vec![(0, 2), (2, 1)]),
                    ("a b", vec![(0, 1), (1, 2)]),
                    ("a!b", vec![(0, 1), (1, 2)]),
                    ("abc", vec![(0, 1), (1, 1), (2, 1)]),
                ] {
                    for reverse in [false, true] {
                        let mut ranges = spans
                            .iter()
                            .map(|&(offset, length)| entity(offset, length))
                            .collect::<Vec<_>>();
                        if reverse {
                            ranges.reverse();
                        }
                        let generated = generate_markdown_message(text, &ranges);
                        assert_eq!(
                            parse_markdown_message(&generated),
                            (
                                text.into(),
                                vec![entity(0, text.encode_utf16().count() as i32)]
                            ),
                            "{generated:?}"
                        );
                    }
                }
                let separate = [entity(0, 1), entity(2, 1)];
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message("a b", &separate)),
                    ("a b".into(), separate.to_vec())
                );
            }
        }),
        ("markdown_adjacent_emphasis_preserves_links", || {
            for italic in [false, true] {
                let style = |offset, length| -> E {
                    if italic {
                        t::MessageEntityItalic { offset, length }.into()
                    } else {
                        t::MessageEntityBold { offset, length }.into()
                    }
                };
                for mention in [false, true] {
                    for (text, link_offset, link_length, spans, expected_spans) in [
                        ("abc", 0, 2, vec![(1, 1), (2, 1)], vec![(1, 1), (2, 1)]),
                        (
                            "abcd",
                            0,
                            2,
                            vec![(1, 1), (2, 1), (3, 1)],
                            vec![(1, 1), (2, 2)],
                        ),
                        (
                            "abc",
                            0,
                            2,
                            vec![(0, 1), (1, 1), (2, 1)],
                            vec![(0, 2), (2, 1)],
                        ),
                        (
                            "abcd",
                            1,
                            2,
                            vec![(0, 1), (1, 1), (2, 1), (3, 1)],
                            vec![(0, 1), (1, 2), (3, 1)],
                        ),
                    ] {
                        let link: E = if mention {
                            t::MessageEntityMentionName {
                                offset: link_offset,
                                length: link_length,
                                user_id: 42,
                            }
                            .into()
                        } else {
                            t::MessageEntityTextUrl {
                                offset: link_offset,
                                length: link_length,
                                url: "https://example.com".into(),
                            }
                            .into()
                        };
                        for reverse in [false, true] {
                            let mut ranges = spans
                                .iter()
                                .map(|&(offset, length)| style(offset, length))
                                .collect::<Vec<_>>();
                            if reverse {
                                ranges.reverse();
                            }
                            ranges.insert(0, link.clone());
                            let generated = generate_markdown_message(text, &ranges);
                            let (parsed, mut actual) = parse_markdown_message(&generated);
                            assert_eq!(parsed, text, "{generated:?}");
                            let mut expected = expected_spans
                                .iter()
                                .map(|&(offset, length)| style(offset, length))
                                .collect::<Vec<_>>();
                            expected.push(link.clone());
                            let order = |entity: &E| {
                                (
                                    entity.offset(),
                                    std::cmp::Reverse(entity.length()),
                                    if matches!(entity, E::TextUrl(_) | E::MentionName(_)) {
                                        0
                                    } else {
                                        1
                                    },
                                )
                            };
                            actual.sort_by_key(order);
                            expected.sort_by_key(order);
                            assert_eq!(actual, expected, "{generated:?}");
                        }
                    }
                }
                let ranges = [
                    style(0, 1),
                    style(1, 1),
                    t::MessageEntityUnderline {
                        offset: 0,
                        length: 1,
                    }
                    .into(),
                ];
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message("ab", &ranges)),
                    ("ab".into(), vec![style(0, 2)])
                );
                let omitted: E = if italic {
                    t::MessageEntityBold {
                        offset: 0,
                        length: 1,
                    }
                    .into()
                } else {
                    t::MessageEntityItalic {
                        offset: 0,
                        length: 1,
                    }
                    .into()
                };
                let ranges = [omitted, style(0, 1), style(1, 2)];
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message("!ab", &ranges)),
                    ("!ab".into(), vec![style(0, 3)])
                );
            }
        }),
        ("markdown_link_destination_entities", || {
            for url in [
                "https://example.com/?x=&copy;",
                "https://example.com/?x=&amp;&y=&#65;",
                "https://example.com/?x=&#x41;&plain=a&b=c",
                "https://example.com/?x=&notAnEntity;",
                "https://example.com/?x=&copy; (a)",
            ] {
                let expected: E = t::MessageEntityTextUrl {
                    offset: 2,
                    length: 1,
                    url: url.into(),
                }
                .into();
                let generated = generate_markdown_message("😀x", std::slice::from_ref(&expected));
                assert_eq!(
                    parse_markdown_message(&generated),
                    ("😀x".into(), vec![expected]),
                    "{generated:?}"
                );
            }
        }),
        ("markdown_multiline_code", || {
            for (text, expected) in [
                ("a\nb", vec![(0, 1), (2, 1)]),
                ("a\r\nb", vec![(0, 1), (3, 1)]),
                ("a\rb", vec![(0, 1), (2, 1)]),
                ("😀\n`b", vec![(0, 2), (3, 2)]),
                ("\na\n\nb\n", vec![(1, 1), (4, 1)]),
            ] {
                let entities = expected
                    .into_iter()
                    .map(|(offset, length)| t::MessageEntityCode { offset, length }.into())
                    .collect::<Vec<E>>();
                let generated = generate_markdown_message(
                    text,
                    &[t::MessageEntityCode {
                        offset: 0,
                        length: text.encode_utf16().count() as i32,
                    }
                    .into()],
                );
                assert_eq!(
                    parse_markdown_message(&generated),
                    (text.into(), entities),
                    "{generated:?}"
                );
            }
        }),
        ("markdown_punctuation_flanking", || {
            for italic in [false, true] {
                let entity = |offset, length| -> E {
                    if italic {
                        t::MessageEntityItalic { offset, length }.into()
                    } else {
                        t::MessageEntityBold { offset, length }.into()
                    }
                };
                for symbol in ["!", "!?", "،", "€", "😀"] {
                    let text = format!("a{symbol}b");
                    let generated = generate_markdown_message(
                        &text,
                        &[entity(1, symbol.encode_utf16().count() as i32)],
                    );
                    assert_eq!(
                        parse_markdown_message(&generated),
                        (text, vec![]),
                        "{generated:?}"
                    );
                    let text = format!("({symbol})");
                    let expected = entity(1, symbol.encode_utf16().count() as i32);
                    let generated =
                        generate_markdown_message(&text, std::slice::from_ref(&expected));
                    assert_eq!(
                        parse_markdown_message(&generated),
                        (text, vec![expected]),
                        "{generated:?}"
                    );
                }
                // A combining mark is neither punctuation nor whitespace;
                // CommonMark can represent its intraword emphasis with '*'.
                let expected = entity(1, 1);
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message(
                        "a\u{301}b",
                        std::slice::from_ref(&expected)
                    )),
                    ("a\u{301}b".into(), vec![expected])
                );
            }
        }),
        ("markdown_plain_line_endings", || {
            for text in [
                "\nhello\n",
                "\rhello\r",
                "\r\nhello\r\n",
                "one\n\n\nthree",
                "\n\n",
                "\r\n",
                "\na\n\nb\n",
                "\n hello \n",
            ] {
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message(text, &[])),
                    (text.into(), vec![])
                );
            }
            let text = "first\n\nlast\n";
            let expected: E = t::MessageEntityPre {
                offset: 0,
                length: text.len() as i32,
                language: "rust".into(),
            }
            .into();
            assert_eq!(
                parse_markdown_message(&generate_markdown_message(
                    text,
                    std::slice::from_ref(&expected)
                )),
                (text.into(), vec![expected])
            );
            let text = "first\n\npre\nlast\n\n";
            let expected: E = t::MessageEntityPre {
                offset: 7,
                length: 4,
                language: "rust".into(),
            }
            .into();
            assert_eq!(
                parse_markdown_message(&generate_markdown_message(
                    text,
                    std::slice::from_ref(&expected)
                )),
                (text.into(), vec![expected])
            );
        }),
        ("markdown_leading_whitespace", || {
            for text in [
                "    hello",
                "\thello",
                "a\n    hello",
                " \t hello",
                "    ",
                "hello  \nworld\t ",
            ] {
                assert_eq!(
                    parse_markdown_message(&generate_markdown_message(text, &[])),
                    (text.into(), vec![])
                );
            }
        }),
        ("markdown_link_destination", || {
            for url in [
                "https://example.com/a)b",
                "https://example.com/a(b",
                "https://example.com/a b",
                "https://example.com/a\\b",
            ] {
                let entity = t::MessageEntityTextUrl {
                    offset: 0,
                    length: 1,
                    url: url.into(),
                }
                .into();
                let parsed = parse_markdown_message(&generate_markdown_message("x", &[entity]));
                assert_eq!(parsed.0, "x");
                assert!(
                    matches!(&parsed.1[0],E::TextUrl(link) if link.url==url),
                    "{url}: {parsed:?}"
                );
            }
        }),
        ("markdown_intraword_italic", || {
            let expected: E = t::MessageEntityItalic {
                offset: 1,
                length: 1,
            }
            .into();
            assert_eq!(
                parse_markdown_message(&generate_markdown_message(
                    "axb",
                    std::slice::from_ref(&expected)
                )),
                ("axb".into(), vec![expected])
            );
        }),
        ("markdown_pre_mid_line", || {
            let entity = t::MessageEntityPre {
                offset: 1,
                length: 1,
                language: "rust".into(),
            }
            .into();
            let generated = generate_markdown_message("aXb", &[entity]);
            assert!(
                generated.starts_with("a\n\n```rust\nX\n```\n"),
                "{generated:?}"
            );
            let parsed = parse_markdown_message(&generated);
            assert!(
                parsed
                    .1
                    .iter()
                    .any(|entity| matches!(entity,E::Pre(pre) if pre.language=="rust")),
                "{parsed:?}"
            );
        }),
        ("markdown_invalid_mention", || {
            let (text, entities) = parse_markdown_message("[x](tg://user?id=abc)");
            assert_eq!(text, "x");
            assert!(
                matches!(&entities[0],E::TextUrl(link) if link.url=="tg://user?id=abc" && link.length==1)
            );
        }),
        ("markdown_literal", || {
            let text = "*literal* 😀 x";
            let entity = t::MessageEntityBold {
                offset: 13,
                length: 1,
            }
            .into();
            let generated = generate_markdown_message(text, &[entity]);
            let parsed = parse_markdown_message(&generated);
            assert_eq!(parsed.0, text);
            assert_eq!(parsed.1.len(), 1);
            assert!(matches!(parsed.1[0], E::Bold(_)));
        }),
        ("markdown_content_space", || {
            let (text, entities) = parse_markdown_message("`x `");
            assert_eq!(text, "x ");
            assert!(matches!(&entities[0],E::Code(code) if code.length==2));
        }),
        ("markdown_soft_break", || {
            assert_eq!(parse_markdown_message("one\ntwo").0, "one\ntwo");
        }),
        ("markdown_code_delimiters", || {
            for text in ["a`b", "`", " a ", "``a`b``"] {
                let generated = generate_markdown_message(
                    text,
                    &[t::MessageEntityCode {
                        offset: 0,
                        length: text.len() as i32,
                    }
                    .into()],
                );
                assert_eq!(parse_markdown_message(&generated).0, text);
            }
        }),
        ("markdown_fence", || {
            for language in ["", "rust"] {
                let generated = generate_markdown_message(
                    "x",
                    &[t::MessageEntityPre {
                        offset: 0,
                        length: 1,
                        language: language.into(),
                    }
                    .into()],
                );
                assert!(generated.contains("\nx\n```"), "{generated}");
                let parsed = parse_markdown_message(&generated);
                assert!(parsed.0.starts_with("x\n"));
                assert!(matches!(&parsed.1[0],E::Pre(code) if code.language==language));
            }
        }),
        ("html_invalid_mention", || {
            let (text, entities) = parse_html_message("<a href=\"tg://user?id=abc\">x</a>");
            assert_eq!(text, "x");
            assert!(matches!(&entities[0],E::TextUrl(link) if link.url=="tg://user?id=abc"));
        }),
        ("html_literal", || {
            let text = "😀 <literal>&";
            let generated = generate_html_message(
                text,
                &[t::MessageEntityBold {
                    offset: 3,
                    length: 10,
                }
                .into()],
            );
            let parsed = parse_html_message(&generated);
            assert_eq!(parsed.0, text);
            assert!(matches!(&parsed.1[0],E::Bold(bold) if bold.offset==3 && bold.length==10));
        }),
        ("html_attribute", || {
            let url = "https://example.org/?q=\"<&";
            let generated = generate_html_message(
                "x",
                &[t::MessageEntityTextUrl {
                    offset: 0,
                    length: 1,
                    url: url.into(),
                }
                .into()],
            );
            let parsed = parse_html_message(&generated);
            assert_eq!(parsed.0, "x");
            assert!(matches!(&parsed.1[0],E::TextUrl(link) if link.url==url));
        }),
        ("html_nested_mention", || {
            let (_, entities) = parse_html_message("<a href=\"tg://user?id=1\"><b>x</b></a>");
            assert!(matches!(&entities[0],E::MentionName(mention) if mention.length==1));
        }),
        ("html_nested_same_kind", || {
            let (text, entities) = parse_html_message("<b>a<strong>b</strong>c</b>");
            assert_eq!(text, "abc");
            assert!(matches!(&entities[0],E::Bold(bold) if bold.length==3));
            assert!(matches!(&entities[1],E::Bold(bold) if bold.offset==1 && bold.length==1));
        }),
        ("html_date_overflow", || {
            for date in ["2040-01-01T00:00:00Z", "1800-01-01T00:00:00Z"] {
                let (text, entities) =
                    parse_html_message(&format!("<time datetime=\"{date}\">x</time>"));
                assert_eq!(text, "x");
                assert!(entities.is_empty());
            }
        }),
        ("mention_digit_boundaries", || {
            for user_id in [0, i64::MIN, i64::MAX] {
                let generated = generate_html_message(
                    "x",
                    &[t::MessageEntityMentionName {
                        offset: 0,
                        length: 1,
                        user_id,
                    }
                    .into()],
                );
                assert_eq!(parse_html_message(&generated).0, "x");
            }
        }),
    ];
    let failures = probes
        .iter()
        .filter_map(|(name, probe)| std::panic::catch_unwind(*probe).err().map(|_| *name))
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "SDK parser regressions: {failures:?}");
}
