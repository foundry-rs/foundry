/// Memory view widget
pub mod memory {
    use revm::Memory;
    use std::collections::HashMap;
    use tui::{
        buffer::Buffer,
        layout::Rect,
        style::{Color, Modifier, Style},
        text::{Span, Spans},
        widgets::{Block, Paragraph, StatefulWidget, Widget, Wrap},
    };

    pub struct MemoryView<'a> {
        /// The memory to display
        memory: &'a Memory,
        /// Words to highlight
        highlights: HashMap<usize, Color>,
        block: Option<Block<'a>>,
    }

    impl<'a> MemoryView<'a> {
        pub fn new(memory: &'a Memory) -> Self {
            Self { memory, highlights: HashMap::new(), block: None }
        }

        pub fn block(mut self, block: Block<'a>) -> Self {
            self.block = Some(block);
            self
        }

        pub fn highlight(mut self, word: usize, color: Color) -> Self {
            self.highlights.insert(word, color);
            self
        }
    }

    #[derive(Default)]
    pub struct MemoryViewState {
        /// The first word in memory to display
        start: usize,
        /// The current view mode
        mode: ViewMode,
    }

    impl MemoryViewState {
        pub fn scroll_to_top(&mut self) {
            self.start = 0;
        }

        pub fn scroll_down(&mut self) {
            self.start = self.start.saturating_add(1);
        }

        pub fn scroll_up(&mut self) {
            self.start = self.start.saturating_sub(1);
        }

        pub fn toggle_mode(&mut self, a: ViewMode, b: ViewMode) {
            if self.mode == a {
                self.mode = b;
            } else {
                self.mode = a;
            }
        }
    }

    impl<'a> StatefulWidget for MemoryView<'a> {
        type State = MemoryViewState;

        fn render(mut self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
            // TODO: Wrap out of bounds `state.start` between 0 and `self.memory.len()`
            let area = match self.block.take() {
                Some(b) => {
                    let inner_area = b.inner(area);
                    b.render(area, buf);
                    inner_area
                }
                None => area,
            };

            // TODO: ???
            let max_i = self.memory.len() / 32;
            let min_len = format!("{:x}", max_i * 32).len();

            let mut graphemes = Vec::new();
            for (i, word) in self.memory.data().chunks(32).enumerate().skip(state.start) {
                let color = self.highlights.get(&i);
                let bytes: Vec<Span> = word
                    .iter()
                    .map(|byte| {
                        Span::styled(
                            format!("{:02x} ", byte),
                            if let Some(color) = color {
                                Style::default().fg(*color)
                            } else if *byte == 0 {
                                Style::default().add_modifier(Modifier::DIM)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        )
                    })
                    .collect();

                // ???
                let mut spans = vec![Span::styled(
                    format!("{:0min_len$x}| ", i * 32, min_len = min_len),
                    Style::default().fg(Color::White),
                )];
                spans.extend(bytes);

                if matches!(state.mode, ViewMode::Utf8) {
                    let chars: Vec<Span> = word
                        .chunks(4)
                        .map(|utf| {
                            if let Ok(s) = std::str::from_utf8(utf) {
                                Span::raw(s.replace(char::from(0), "."))
                            } else {
                                Span::raw(".")
                            }
                        })
                        .collect();
                    spans.push(Span::raw("|"));
                    spans.extend(chars);
                }

                graphemes.push(Spans::from(spans));
            }

            Paragraph::new(graphemes).wrap(Wrap { trim: true }).render(area, buf);
        }
    }

    /// The view mode for memory words.
    #[derive(Default, Eq, PartialEq)]
    pub enum ViewMode {
        /// Display words as hexadecimal
        #[default]
        Hex,
        /// Display words as UTF8
        Utf8,
    }
}
