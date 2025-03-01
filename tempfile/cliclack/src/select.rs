use std::cell::RefCell;
use std::io;
use std::{fmt::Display, rc::Rc};

use console::Key;

use crate::{
    filter::{FilteredView, LabeledItem},
    prompt::{
        cursor::StringCursor,
        interaction::{Event, PromptInteraction, State},
    },
    theme::THEME,
};

#[derive(Clone)]
struct RadioButton<T> {
    value: T,
    label: String,
    hint: String,
}

impl<T> LabeledItem for RadioButton<T> {
    fn label(&self) -> &str {
        &self.label
    }
}

/// A prompt that asks for one selection from a list of options.
pub struct Select<T> {
    prompt: String,
    items: Vec<Rc<RefCell<RadioButton<T>>>>,
    cursor: usize,
    initial_value: Option<T>,
    filter: FilteredView<RadioButton<T>>,
}

impl<T> Select<T>
where
    T: Clone + Eq,
{
    /// Creates a new selection prompt.
    pub fn new(prompt: impl Display) -> Self {
        Self {
            prompt: prompt.to_string(),
            items: Vec::new(),
            cursor: 0,
            initial_value: None,
            filter: FilteredView::default(),
        }
    }

    /// Adds an item to the selection prompt.
    pub fn item(mut self, value: T, label: impl Display, hint: impl Display) -> Self {
        self.items.push(Rc::new(RefCell::new(RadioButton {
            value,
            label: label.to_string(),
            hint: hint.to_string(),
        })));
        self
    }

    /// Adds multiple items to the list of options.
    pub fn items(mut self, items: &[(T, impl Display, impl Display)]) -> Self {
        for (value, label, hint) in items {
            self = self.item(value.clone(), label, hint);
        }
        self
    }

    /// Sets the initially selected item by value.
    pub fn initial_value(mut self, value: T) -> Self {
        self.initial_value = Some(value);
        self
    }

    /// Enables the filter mode ("fuzzy search").
    ///
    /// The filter mode allows to filter the items by typing.
    pub fn filter_mode(mut self) -> Self {
        self.filter.enable();
        self
    }

    /// Starts the prompt interaction.
    pub fn interact(&mut self) -> io::Result<T> {
        if self.items.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No items added to the list",
            ));
        }
        if let Some(initial_value) = &self.initial_value {
            self.cursor = self
                .items
                .iter()
                .position(|item| item.borrow().value == *initial_value)
                .unwrap_or(self.cursor);
        }
        self.filter.set(self.items.to_vec());
        <Self as PromptInteraction<T>>::interact(self)
    }
}

impl<T: Clone> PromptInteraction<T> for Select<T> {
    fn on(&mut self, event: &Event) -> State<T> {
        let Event::Key(key) = event;

        if let Some(state) = self.filter.on(key, self.items.clone()) {
            if self.filter.items().is_empty() || self.cursor > self.filter.items().len() - 1 {
                self.cursor = 0;
            }
            return state;
        }

        match key {
            Key::ArrowUp | Key::ArrowLeft | Key::Char('k') | Key::Char('h') => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            Key::ArrowDown | Key::ArrowRight | Key::Char('j') | Key::Char('l') => {
                if !self.filter.items().is_empty() && self.cursor < self.filter.items().len() - 1 {
                    self.cursor += 1;
                }
            }
            Key::Enter => {
                return State::Submit(self.filter.items()[self.cursor].borrow().value.clone());
            }
            _ => {}
        }

        State::Active
    }

    fn render(&mut self, state: &State<T>) -> String {
        let theme = THEME.lock().unwrap();

        let header_display = theme.format_header(&state.into(), &self.prompt);
        let footer_display = theme.format_footer(&state.into());

        let filter_display = if let Some(input) = &self.filter.input() {
            match state {
                State::Submit(_) | State::Cancel => "".to_string(),
                _ => theme.format_input(&state.into(), input),
            }
        } else {
            "".to_string()
        };

        let items_display: String = self
            .filter
            .items()
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let item = item.borrow();
                theme.format_select_item(&state.into(), self.cursor == i, &item.label, &item.hint)
            })
            .collect();

        header_display + &filter_display + &items_display + &footer_display
    }

    /// Enable handling of the input in the filter mode.
    fn input(&mut self) -> Option<&mut StringCursor> {
        self.filter.input()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn empty_list() {
        let mut select = Select::<&str>::new("Select an item").initial_value("");
        let result = select.interact();
        assert_eq!(
            "No items added to the list",
            result.unwrap_err().to_string()
        );
    }
}
