use crate::app::Message;
use codec::{CadDocument, Handle};
use iced::{
    widget::{button, checkbox, column, container, row, scrollable, text, Space},
    Element, Fill,
};

pub(crate) struct Row {
    pub handle: Handle,
    pub label: String,
    pub enabled: bool,
}

pub(crate) struct State {
    pub document: u64,
    pub definition: Handle,
    pub expected: Vec<(Handle, codec::objects::ObjectType)>,
    pub rows: Vec<Row>,
    pub error: Option<String>,
}

impl State {
    pub(crate) fn new(
        document: u64,
        definition: Handle,
        doc: &CadDocument,
    ) -> Result<Self, &'static str> {
        let grips = crate::scene::block_grips::grips(doc, definition)?;
        if grips.is_empty() {
            return Err("This definition has no native grips.");
        }
        let expected = grips
            .iter()
            .map(|g| (g.handle, doc.objects[&g.handle].clone()))
            .collect();
        let rows = grips
            .into_iter()
            .map(|g| Row {
                handle: g.handle,
                enabled: g.enabled,
                label: format!(
                    "{} [{:X}]   ({:.4}, {:.4}, {:.4})",
                    g.name,
                    g.handle.value(),
                    g.point.x,
                    g.point.y,
                    g.point.z
                ),
            })
            .collect();
        Ok(Self {
            document,
            definition,
            expected,
            rows,
            error: None,
        })
    }
}

pub(crate) fn view_window(state: &State) -> Element<'_, Message> {
    let mut rows = column![].spacing(8);
    for (index, grip) in state.rows.iter().enumerate() {
        let mut up = button(text(crate::t!("Up")));
        let mut down = button(text(crate::t!("Down")));
        if index > 0 {
            up = up.on_press(Message::BlockCycleMove(index, true));
        }
        if index + 1 < state.rows.len() {
            down = down.on_press(Message::BlockCycleMove(index, false));
        }
        rows = rows.push(
            row![
                checkbox(grip.enabled)
                    .label(grip.label.clone())
                    .on_toggle(move |enabled| Message::BlockCycleEnable(index, enabled))
                    .width(Fill),
                up,
                down
            ]
            .spacing(8)
            .align_y(iced::Center),
        );
    }
    let mut content=column![
        text(crate::t!("Enable insertion grips and arrange their cycle order. Press Ctrl during insertion to select the next enabled point.")),
        scrollable(rows).height(360),
    ].spacing(12);
    if let Some(error) = &state.error {
        content = content.push(text(error).style(iced::widget::text::danger));
    }
    content = content.push(
        row![
            Space::new().width(Fill),
            button(text(crate::t!("Cancel"))).on_press(Message::CloseModal),
            button(text(crate::t!("Apply"))).on_press(Message::BlockCycleApply)
        ]
        .spacing(8),
    );
    container(content).padding(18).width(740).into()
}
