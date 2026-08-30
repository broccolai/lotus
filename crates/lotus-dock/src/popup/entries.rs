use super::{
    APP_ROW_DIP, APP_WIDTH_DIP, COMPACT_ROW_DIP, COMPACT_WIDTH_DIP, DockPopup, GAP_DIP,
    MAX_COMPACT_ROWS, MAX_THUMBNAIL_CARDS, NonZeroPhysicalSize, PADDING_DIP, POWER_ROW_DIP,
    POWER_WIDTH_DIP, PhysicalRect, PhysicalUnsignedPoint, PickerWindow, PopupAction,
    PopupEntry, PopupIcon, PopupKind, PopupSymbol, THUMBNAIL_CARD_HEIGHT_DIP,
    THUMBNAIL_CARD_WIDTH_DIP, THUMBNAIL_HEADER_DIP, WindowPickerStyle, physical_rect,
    power_entries, system_label, system_symbol,
};

impl<Asset: Clone> DockPopup<Asset> {
    pub fn desired_size(&self) -> NonZeroPhysicalSize {
        match &self.kind {
            PopupKind::System(menu) => menu.desired_size(),
            PopupKind::Power => self.vertical_size(POWER_WIDTH_DIP, POWER_ROW_DIP, 4),
            PopupKind::FileLocation(_) => self.vertical_size(APP_WIDTH_DIP, APP_ROW_DIP, 1),
            PopupKind::App { entries, .. } => NonZeroPhysicalSize::new(
                self.scale.physical(APP_WIDTH_DIP),
                self.scale.physical(
                    PADDING_DIP * 2
                        + APP_ROW_DIP * u32::try_from(entries.len()).unwrap_or(u32::MAX)
                        + GAP_DIP
                            * u32::try_from(entries.len().saturating_sub(1))
                                .unwrap_or(u32::MAX),
                ),
            )
            .expect("app popup dimensions are nonzero"),
            PopupKind::Picker { style, entries, .. } => {
                self.picker_size(*style, entries.len())
            }
        }
    }

    pub fn entries(&self) -> Vec<PopupEntry<Asset>> {
        match &self.kind {
            PopupKind::System(menu) => menu
                .items()
                .into_iter()
                .enumerate()
                .map(|(index, (action, bounds))| PopupEntry {
                    action: PopupAction::System(action),
                    label: system_label(action).to_owned(),
                    icon: PopupIcon::Symbol(system_symbol(action)),
                    bounds,
                    preview: None,
                    close: None,
                    active: false,
                    highlighted: self.highlighted(index),
                    close_highlighted: false,
                })
                .collect(),
            PopupKind::Power => power_entries()
                .into_iter()
                .enumerate()
                .map(|(index, (action, label, symbol))| PopupEntry {
                    action: PopupAction::Power(action),
                    label: label.to_owned(),
                    icon: PopupIcon::Symbol(symbol),
                    bounds: self.vertical_row(index, POWER_WIDTH_DIP, POWER_ROW_DIP),
                    preview: None,
                    close: None,
                    active: false,
                    highlighted: self.highlighted(index),
                    close_highlighted: false,
                })
                .collect(),
            PopupKind::FileLocation(path) => vec![PopupEntry {
                action: PopupAction::OpenFileLocation(path.clone()),
                label: "Open file location".to_owned(),
                icon: PopupIcon::Symbol(PopupSymbol::Open),
                bounds: self.vertical_row(0, APP_WIDTH_DIP, APP_ROW_DIP),
                preview: None,
                close: None,
                active: false,
                highlighted: self.highlighted(0),
                close_highlighted: false,
            }],
            PopupKind::App {
                identity, entries, ..
            } => entries
                .iter()
                .enumerate()
                .map(|(index, entry)| PopupEntry {
                    action: PopupAction::App {
                        action: entry.action,
                        identity: identity.clone(),
                    },
                    label: entry.label.to_owned(),
                    icon: PopupIcon::Symbol(entry.symbol),
                    bounds: self.vertical_row(index, APP_WIDTH_DIP, APP_ROW_DIP),
                    preview: None,
                    close: None,
                    active: false,
                    highlighted: self.highlighted(index),
                    close_highlighted: false,
                })
                .collect(),
            PopupKind::Picker { style, entries, .. } => {
                self.picker_entries(*style, entries)
            }
        }
    }

    fn picker_entries(
        &self,
        style: WindowPickerStyle,
        entries: &[PickerWindow<Asset>],
    ) -> Vec<PopupEntry<Asset>> {
        let visible = match style {
            WindowPickerStyle::Compact => MAX_COMPACT_ROWS,
            WindowPickerStyle::Thumbnails => MAX_THUMBNAIL_CARDS,
        };
        entries
            .iter()
            .skip(self.offset)
            .take(visible)
            .enumerate()
            .map(|(visual, entry)| {
                let index = self.offset + visual;
                let (bounds, preview, close) = match style {
                    WindowPickerStyle::Compact => {
                        let bounds =
                            self.vertical_row(visual, COMPACT_WIDTH_DIP, COMPACT_ROW_DIP);
                        let close_size = self.scale.physical(24);
                        let close = physical_rect(
                            bounds
                                .max_x()
                                .saturating_sub(close_size + self.scale.physical(8)),
                            bounds.min_y().saturating_add(
                                bounds.height().saturating_sub(close_size) / 2,
                            ),
                            close_size,
                            close_size,
                        );
                        (bounds, None, Some(close))
                    }
                    WindowPickerStyle::Thumbnails => {
                        let padding = self.scale.physical(PADDING_DIP);
                        let gap = self.scale.physical(GAP_DIP);
                        let width = self.scale.physical(THUMBNAIL_CARD_WIDTH_DIP);
                        let height = self.scale.physical(THUMBNAIL_CARD_HEIGHT_DIP);
                        let left = padding.saturating_add(
                            u32::try_from(visual)
                                .unwrap_or(u32::MAX)
                                .saturating_mul(width.saturating_add(gap)),
                        );
                        let bounds = physical_rect(left, padding, width, height);
                        let header = self.scale.physical(THUMBNAIL_HEADER_DIP);
                        let inset = self.scale.physical(4);
                        let preview = physical_rect(
                            left.saturating_add(inset),
                            padding.saturating_add(header),
                            width.saturating_sub(inset * 2),
                            height.saturating_sub(header + inset),
                        );
                        let close_size = self.scale.physical(24);
                        let close = physical_rect(
                            bounds.max_x().saturating_sub(close_size + inset),
                            bounds.min_y().saturating_add(inset),
                            close_size,
                            close_size,
                        );
                        (bounds, Some(preview), Some(close))
                    }
                };
                let close_highlighted = self.hovered == Some((index, true));
                PopupEntry {
                    action: PopupAction::Activate(entry.key),
                    label: entry.title.clone(),
                    icon: PopupIcon::Artwork(entry.icon.clone()),
                    bounds,
                    preview,
                    close,
                    active: entry.active,
                    highlighted: self.highlighted(index),
                    close_highlighted,
                }
            })
            .collect()
    }

    fn picker_size(&self, style: WindowPickerStyle, count: usize) -> NonZeroPhysicalSize {
        let visible = match style {
            WindowPickerStyle::Compact => count.clamp(1, MAX_COMPACT_ROWS),
            WindowPickerStyle::Thumbnails => count.clamp(1, MAX_THUMBNAIL_CARDS),
        };
        let visible = u32::try_from(visible).unwrap_or(u32::MAX);
        let (width, height) = match style {
            WindowPickerStyle::Compact => (
                COMPACT_WIDTH_DIP,
                PADDING_DIP * 2 + visible * COMPACT_ROW_DIP + (visible - 1) * GAP_DIP,
            ),
            WindowPickerStyle::Thumbnails => (
                PADDING_DIP * 2
                    + visible * THUMBNAIL_CARD_WIDTH_DIP
                    + (visible - 1) * GAP_DIP,
                PADDING_DIP * 2 + THUMBNAIL_CARD_HEIGHT_DIP,
            ),
        };
        NonZeroPhysicalSize::new(self.scale.physical(width), self.scale.physical(height))
            .expect("picker popup dimensions are nonzero")
    }

    fn vertical_row(
        &self,
        index: usize,
        width_dips: u32,
        height_dips: u32,
    ) -> PhysicalRect {
        let padding = self.scale.physical(PADDING_DIP);
        let gap = self.scale.physical(GAP_DIP);
        let height = self.scale.physical(height_dips);
        let top = padding.saturating_add(
            u32::try_from(index)
                .unwrap_or(u32::MAX)
                .saturating_mul(height.saturating_add(gap)),
        );
        physical_rect(
            padding,
            top,
            self.scale.physical(width_dips).saturating_sub(padding * 2),
            height,
        )
    }

    fn vertical_size(
        &self,
        width_dips: u32,
        row_dips: u32,
        count: u32,
    ) -> NonZeroPhysicalSize {
        NonZeroPhysicalSize::new(
            self.scale.physical(width_dips),
            self.scale.physical(
                PADDING_DIP * 2 + row_dips * count + GAP_DIP * count.saturating_sub(1),
            ),
        )
        .expect("popup dimensions are nonzero")
    }

    fn highlighted(&self, index: usize) -> bool {
        self.hovered.is_some_and(|hovered| hovered.0 == index)
            || self.selected == Some(index)
    }

    pub(super) fn entry_at(&self, x: i32, y: i32) -> Option<(usize, bool)> {
        let point =
            PhysicalUnsignedPoint::new(u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        self.entries()
            .iter()
            .enumerate()
            .find_map(|(visual, entry)| {
                let index = self.offset + visual;
                entry
                    .close
                    .filter(|close| close.contains(point))
                    .map(|_| (index, true))
                    .or_else(|| entry.bounds.contains(point).then_some((index, false)))
            })
    }

    pub(super) fn picker_extent(&self) -> Option<(usize, usize)> {
        let PopupKind::Picker { style, entries, .. } = &self.kind else {
            return None;
        };
        Some((
            match style {
                WindowPickerStyle::Compact => MAX_COMPACT_ROWS,
                WindowPickerStyle::Thumbnails => MAX_THUMBNAIL_CARDS,
            },
            entries.len(),
        ))
    }

    pub(super) fn keep_selection_visible(&mut self) {
        let Some((visible, total)) = self.picker_extent() else {
            return;
        };
        let Some(selected) = self.selected else {
            return;
        };
        if selected < self.offset {
            self.offset = selected;
        } else if selected >= self.offset + visible {
            self.offset = selected + 1 - visible;
        }
        self.offset = self.offset.min(total.saturating_sub(visible));
    }
}
