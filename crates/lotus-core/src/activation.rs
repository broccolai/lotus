#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationDecision<Window> {
    Launch,
    Minimize(Window),
    Focus(Window),
}

pub fn decide_activation<Window>(
    windows: &[Window],
    foreground: Option<&Window>,
) -> ActivationDecision<Window>
where
    Window: Copy + Eq,
{
    match windows {
        [] => ActivationDecision::Launch,
        [window] if foreground == Some(window) => ActivationDecision::Minimize(*window),
        [window] => ActivationDecision::Focus(*window),
        multiple => {
            let next_index = foreground
                .and_then(|foreground| multiple.iter().position(|window| window == foreground))
                .map_or(0, |index| (index + 1) % multiple.len());

            ActivationDecision::Focus(multiple[next_index])
        }
    }
}
