#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameTrigger {
    Changes,
    AnimationTick,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameOutcome {
    Complete { continues_animation: bool },
    Retry,
}

impl FrameOutcome {
    pub const fn complete(continues_animation: bool) -> Self {
        Self::Complete {
            continues_animation,
        }
    }
}

pub struct ScheduledSurface<T> {
    value: T,
    dirty: bool,
    animating: bool,
}

impl<T> ScheduledSurface<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            dirty: true,
            animating: false,
        }
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut T {
        self.invalidate();
        &mut self.value
    }

    pub fn replace(&mut self, value: T) -> T {
        self.dirty = true;
        self.animating = false;
        std::mem::replace(&mut self.value, value)
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn stop_animation(&mut self) {
        self.animating = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }
}

pub struct FramePass {
    trigger: FrameTrigger,
    animation_active: bool,
}

impl FramePass {
    pub fn new(trigger: FrameTrigger) -> Self {
        Self {
            trigger,
            animation_active: false,
        }
    }

    pub fn animation_active(&self) -> bool {
        self.animation_active
    }

    pub fn request_next_frame(&mut self) {
        self.animation_active = true;
    }

    pub fn render<T, E, F>(
        &mut self,
        surface: &mut ScheduledSurface<T>,
        render: F,
    ) -> Result<(), E>
    where
        F: FnOnce(&mut T) -> Result<FrameOutcome, E>,
    {
        let should_render = surface.dirty
            || (self.trigger == FrameTrigger::AnimationTick && surface.animating);

        if !should_render {
            self.animation_active |= surface.animating;
            return Ok(());
        }

        match render(&mut surface.value)? {
            FrameOutcome::Complete {
                continues_animation,
            } => {
                surface.dirty = false;
                surface.animating = continues_animation;
                self.animation_active |= continues_animation;
            }
            FrameOutcome::Retry => {
                surface.dirty = true;
                self.animation_active = true;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameOutcome, FramePass, FrameTrigger, ScheduledSurface};

    #[test]
    fn scheduling_state_machine_preserves_dirty_and_animation_demand() {
        let mut surface = ScheduledSurface::new(0);
        assert!(surface.is_dirty());

        let mut pass = FramePass::new(FrameTrigger::Changes);
        pass.render(&mut surface, |value| {
            *value += 1;
            Ok::<_, ()>(FrameOutcome::complete(true))
        })
        .unwrap();
        assert_eq!(*surface.value(), 1);
        assert!(!surface.is_dirty());
        assert!(surface.is_animating());
        assert!(pass.animation_active());

        let mut pass = FramePass::new(FrameTrigger::Changes);
        pass.render(&mut surface, |_| Ok::<_, ()>(FrameOutcome::complete(false)))
            .unwrap();
        assert!(pass.animation_active());
        assert!(surface.is_animating());

        let mut pass = FramePass::new(FrameTrigger::AnimationTick);
        pass.render(&mut surface, |value| {
            *value += 1;
            Ok::<_, ()>(FrameOutcome::complete(false))
        })
        .unwrap();
        assert_eq!(*surface.value(), 2);
        assert!(!surface.is_animating());
        assert!(!pass.animation_active());

        surface.replace(9);
        assert!(surface.is_dirty());
        assert!(!surface.is_animating());

        let mut pass = FramePass::new(FrameTrigger::Changes);
        pass.render(&mut surface, |_| Ok::<_, ()>(FrameOutcome::Retry))
            .unwrap();
        assert!(surface.is_dirty());
        assert!(pass.animation_active());

        assert!(
            FramePass::new(FrameTrigger::Changes)
                .render(&mut surface, |_| Err::<FrameOutcome, _>("failed"))
                .is_err()
        );
        assert!(surface.is_dirty());
        assert!(!surface.is_animating());
    }
}
