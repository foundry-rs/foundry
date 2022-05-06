use forge::abi::CHEATCODE_ADDRESS;
use tui::{
    style::Style,
    text::{Span, Spans},
};

use forge::trace::*;
use foundry_evm::CallKind;

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

pub fn arena_fmt(arena: &'_ CallTraceArena, max: usize, curr: usize) -> Vec<Spans<'_>> {
    #[allow(clippy::too_many_arguments)]
    fn inner<'a>(
        arena: &'a CallTraceArena,
        idx: usize,
        left: &str,
        child_str: &str,
        spans: &mut Vec<Spans<'a>>,
        max: usize,
        curr: usize,
    ) {
        if idx >= max {
            return
        }
        let node = &arena.arena[idx];
        let bold = curr == idx;

        // Display trace header
        let func_header = trace_fmt(&node.trace, left.to_string(), bold).to_owned();
        spans.push(func_header);

        // Display logs and subcalls
        let left_prefix = format!("{child_str}{BRANCH}");
        let right_prefix = format!("{child_str}{PIPE}");

        let mut max_lco_idx = 0;
        // iterate thru the ordering, checking the
        for (lco_idx, lco) in node.ordering.iter().enumerate() {
            match lco {
                LogCallOrder::Log(_index) => {
                    max_lco_idx += 1;
                }
                LogCallOrder::Call(order_idx) => {
                    let child_idx_in_arena = node.children[*order_idx];
                    if child_idx_in_arena < max {
                        max_lco_idx = lco_idx + 1;
                    } else {
                        break
                    }
                }
            }
        }

        for child in &node.ordering[..max_lco_idx] {
            match child {
                LogCallOrder::Log(index) => {
                    spans.extend(log_fmt(&node.logs[*index], child_str, bold).to_owned());
                }
                LogCallOrder::Call(index) => {
                    inner(
                        arena,
                        node.children[*index],
                        &left_prefix,
                        &right_prefix,
                        spans,
                        max,
                        curr,
                    );
                }
            }
        }

        // Display trace return data
        if max_lco_idx == node.ordering.len() {
            let color = style_trace_color(&node.trace);
            let mut s = vec![];
            s.push(Span::raw(format!("{}{}", child_str, EDGE)));
            s.push(Span::styled(RETURN.to_string(), color));
            if node.trace.created() {
                if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                    s.push(Span::raw(format!("{} bytes of code\n", bytes.len())));
                } else {
                    unreachable!("We should never have decoded calldata for contract creations");
                }
            } else {
                s.push(ret_data_fmt(&node.trace.output, bold).to_owned());
            }
            spans.push(Spans::from(s));
        }
    }

    let mut spans = vec![];
    inner(arena, 0, "  ", "  ", &mut spans, max, curr);
    spans
}

pub fn log_fmt<'a>(log: &'a RawOrDecodedLog, child: &str, bold: bool) -> Vec<Spans<'a>> {
    let left_prefix = format!("{child}{BRANCH}");
    let right_prefix = format!("{child}{PIPE}");

    let mut spans = match log {
        RawOrDecodedLog::Raw(log) => {
            let mut spans = vec![];
            for (i, topic) in log.topics.iter().enumerate() {
                let mut s = vec![];
                s.push(Span::raw(format!(
                    "{}{:>13}: ",
                    if i == 0 { &left_prefix } else { &right_prefix },
                    if i == 0 { "emit topic 0".to_string() } else { format!("topic {i}") }
                )));
                s.push(Span::styled(
                    format!("0x{}\n", hex::encode(&topic)),
                    Style::default().fg(tui::style::Color::Cyan),
                ));
                spans.push(Spans::from(s));
            }
            spans.push(Spans::from(Span::styled(
                format!("          data: 0x{}", hex::encode(&log.data)),
                Style::default().fg(tui::style::Color::Cyan),
            )));
            spans
        }
        RawOrDecodedLog::Decoded(name, params) => {
            let params = params
                .iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<String>>()
                .join(", ");
            vec![Spans::from(vec![
                Span::raw(left_prefix),
                Span::styled(
                    format!("emit {}", name.clone()),
                    Style::default().fg(tui::style::Color::Cyan),
                ),
                Span::raw(format!("({})", params)),
            ])]
        }
    };
    if bold {
        spans = spans
            .into_iter()
            .map(|mut spans| {
                for mut span in &mut spans.0 {
                    span.style = span.style.add_modifier(tui::style::Modifier::BOLD);
                }
                spans
            })
            .collect();
    } else {
        spans = spans
            .into_iter()
            .map(|mut spans| {
                for mut span in &mut spans.0 {
                    span.style = span.style.add_modifier(tui::style::Modifier::DIM);
                }
                spans
            })
            .collect();
    }
    spans
}

pub fn ret_data_fmt(retdata: &RawOrDecodedReturnData, bold: bool) -> Span {
    let mut span = match &retdata {
        RawOrDecodedReturnData::Raw(bytes) => {
            if bytes.is_empty() {
                Span::raw("()".to_string())
            } else {
                Span::raw(format!("0x{}", hex::encode(&bytes)))
            }
        }
        RawOrDecodedReturnData::Decoded(decoded) => Span::raw(decoded.clone()),
    };
    if bold {
        span.style = span.style.add_modifier(tui::style::Modifier::BOLD);
    } else {
        span.style = span.style.add_modifier(tui::style::Modifier::DIM);
    }
    span
}

fn trace_fmt(trace: &'_ CallTrace, left: String, bold: bool) -> Spans<'_> {
    let mut spans = if trace.created() {
        Spans::from(vec![
            Span::raw(format!("{}[{}]", left, trace.gas_cost,)),
            Span::styled(
                format!(" {}{} ", CALL, "new",),
                Style::default().fg(tui::style::Color::Yellow),
            ),
            Span::raw(format!(
                "{}@{:?}\n",
                trace.label.as_ref().unwrap_or(&"<Unknown>".to_string()),
                trace.address
            )),
        ])
    } else {
        let (func, inputs) = match &trace.data {
            RawOrDecodedCall::Raw(bytes) => {
                // We assume that the fallback function (`data.len() < 4`) counts as decoded
                // calldata
                assert!(bytes.len() >= 4);
                (hex::encode(&bytes[0..4]), hex::encode(&bytes[4..]))
            }
            RawOrDecodedCall::Decoded(func, inputs) => (func.clone(), inputs.join(", ")),
        };

        let action = match trace.kind {
            // do not show anything for CALLs
            CallKind::Call => "",
            CallKind::StaticCall => "[staticcall]",
            CallKind::CallCode => "[callcode]",
            CallKind::DelegateCall => "[delegatecall]",
            _ => unreachable!(),
        };

        let color = style_trace_color(trace);
        Spans::from(vec![
            Span::raw(format!("{}[{}]", left, trace.gas_cost,)),
            Span::styled(
                format!(
                    " {}::{}",
                    trace.label.as_ref().unwrap_or(&trace.address.to_string()),
                    func
                ),
                color,
            ),
            Span::raw(format!(
                "{}({}) ",
                if !trace.value.is_zero() {
                    format!("{{value: {}}}", trace.value)
                } else {
                    "".to_string()
                },
                inputs,
            )),
            Span::styled(format!("{}\n", action), Style::default().fg(tui::style::Color::Yellow)),
        ])
    };

    if bold {
        for mut span in &mut spans.0 {
            span.style = span.style.add_modifier(tui::style::Modifier::BOLD);
        }
    } else {
        for mut span in &mut spans.0 {
            span.style = span.style.add_modifier(tui::style::Modifier::DIM);
        }
    }
    spans
}

/// Chooses the color of the trace depending on the destination address and status of the call.
fn style_trace_color(trace: &CallTrace) -> Style {
    if trace.address == CHEATCODE_ADDRESS {
        Style::default().fg(tui::style::Color::Cyan)
    } else if trace.success {
        Style::default().fg(tui::style::Color::Green)
    } else {
        Style::default().fg(tui::style::Color::Red)
    }
}
