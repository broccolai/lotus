use super::{
    DockZone, OnboardingModule, OnboardingStep, ROW_HEIGHT_DIP, SettingsAction,
    SettingsControl, SettingsRect, SettingsScene, is_page_content,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct OnboardingState {
    step: Option<OnboardingStep>,
    required: bool,
}

impl OnboardingState {
    pub(super) const fn step(&self) -> Option<OnboardingStep> {
        self.step
    }

    pub(super) const fn required(&self) -> bool {
        self.required
    }

    pub(super) fn begin(&mut self, required: bool) {
        self.step = Some(OnboardingStep::Welcome);
        self.required = required;
    }

    pub(super) fn end(&mut self) {
        self.step = None;
        self.required = false;
    }

    pub(super) fn move_step(&mut self, forward: bool) {
        let Some(step) = self.step else {
            return;
        };
        let number = if forward {
            step.number().saturating_add(1).min(4)
        } else {
            step.number().saturating_sub(1)
        };
        self.step = Some(match number {
            0 => OnboardingStep::Welcome,
            1 => OnboardingStep::Modules,
            2 => OnboardingStep::Layout,
            3 => OnboardingStep::Integration,
            _ => OnboardingStep::Ready,
        });
    }
}

impl SettingsScene {
    pub fn onboarding_module_enabled(&self, module: OnboardingModule) -> bool {
        self.draft.onboarding_module_enabled(module)
    }

    pub const fn onboarding_zone(&self, module: OnboardingModule) -> DockZone {
        self.draft.onboarding_zone(module)
    }

    pub(super) fn cycle_onboarding_zone(
        &mut self,
        module: OnboardingModule,
        reverse: bool,
    ) -> SettingsAction {
        let current = self.onboarding_zone(module);
        let index = DockZone::ALL
            .iter()
            .position(|zone| *zone == current)
            .unwrap_or_default();
        let next = if reverse {
            DockZone::ALL[(index + DockZone::ALL.len() - 1) % DockZone::ALL.len()]
        } else {
            DockZone::ALL[(index + 1) % DockZone::ALL.len()]
        };
        self.set_onboarding_zone(module, next);
        SettingsAction::Changed
    }

    pub(super) fn set_onboarding_zone_from_pointer(
        &mut self,
        module: OnboardingModule,
        bounds: SettingsRect,
        x: u32,
    ) -> SettingsAction {
        let relative = x
            .saturating_sub(bounds.left)
            .min(bounds.width.saturating_sub(1));
        let index = relative.saturating_mul(3) / bounds.width.max(1);
        let zone = DockZone::ALL[usize::try_from(index).unwrap_or_default().min(2)];
        self.set_onboarding_zone(module, zone);
        SettingsAction::Changed
    }

    pub(super) fn set_onboarding_zone(&mut self, module: OnboardingModule, zone: DockZone) {
        self.draft.set_onboarding_zone(module, zone);
    }

    pub(super) fn move_onboarding(&mut self, forward: bool) {
        self.onboarding.move_step(forward);
        self.focused = Some(if self.onboarding.step() == Some(OnboardingStep::Ready) {
            SettingsControl::OnboardingFinish
        } else {
            SettingsControl::OnboardingNext
        });
    }

    pub(super) fn move_focus(&mut self, reverse: bool) {
        let layout = self.layout();
        let focusable: Vec<_> = layout
            .controls
            .into_iter()
            .filter(|entry| {
                !matches!(
                    entry.control,
                    SettingsControl::Apply | SettingsControl::Revert
                ) || self.is_dirty()
            })
            .map(|entry| entry.control)
            .collect();
        if focusable.is_empty() {
            self.focused = None;
            return;
        }
        let current = self
            .focused
            .and_then(|value| focusable.iter().position(|item| *item == value));
        let next = match (current, reverse) {
            (Some(0) | None, true) => focusable.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % focusable.len(),
            (None, false) => 0,
        };
        self.focused = Some(focusable[next]);
        self.reveal_focused_control();
    }

    pub(super) fn reveal_focused_control(&mut self) {
        let Some(control) = self.focused.filter(|control| is_page_content(*control)) else {
            return;
        };
        let Some(top) = self
            .page_content_positions()
            .controls
            .into_iter()
            .find_map(|(item, top)| (item == control).then_some(top))
        else {
            return;
        };

        let bottom = top.saturating_add(ROW_HEIGHT_DIP);
        let viewport_height = Self::content_viewport_height_dip();
        if top < self.scroll_offset_dip {
            self.scroll_offset_dip = top;
        } else if bottom > self.scroll_offset_dip.saturating_add(viewport_height) {
            self.scroll_offset_dip = bottom.saturating_sub(viewport_height);
        }
    }
}
