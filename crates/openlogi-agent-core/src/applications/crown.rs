//! Crown's adapter from saved functions to device-independent application tools.

use std::collections::BTreeMap;

use openlogi_core::crown::{CrownError, CrownFunction, CrownFunctionId, CrownProfile};

use super::{ApplicationContext, ApplicationHost, ControlScope};
use crate::capture_plan::DeviceCapturePlan;

pub(crate) fn apply_crown_contexts(
    host: &ApplicationHost,
    plans: &mut [DeviceCapturePlan],
    focus: Option<&str>,
) {
    let mut allowed = BTreeMap::new();
    for plan in plans {
        let Some(policy) = plan
            .dispatch
            .crown
            .as_mut()
            .filter(|policy| policy.profile.is_enabled())
        else {
            continue;
        };
        let Some(context) = policy
            .profile
            .provider()
            .and_then(|provider| host.resolve(provider))
            .map(|context| context.with_focus(focus))
        else {
            continue;
        };
        match profile_for(&policy.profile, &context) {
            Ok(profile) => {
                allowed.insert(
                    ControlScope::new(plan.dispatch.config_key.clone(), policy.scope.clone()),
                    context.clone(),
                );
                policy.profile = profile;
                policy.application = Some(context);
            }
            Err(error) => {
                tracing::warn!(%error, "application tools failed Crown profile validation");
            }
        }
    }
    host.reconcile(allowed);
}

pub(crate) fn profile_for(
    saved: &CrownProfile,
    context: &ApplicationContext,
) -> Result<CrownProfile, CrownError> {
    let functions = context
        .tools()
        .tools()
        .iter()
        .map(|tool| {
            let id = CrownFunctionId::new(tool.id.as_ref().to_owned())?;
            let mut function = CrownFunction::new(id, tool.name.as_ref(), tool.icon)?;
            function.set_feel(tool.tactile);
            let rotation = saved
                .functions()
                .iter()
                .find(|function| function.id().as_ref() == tool.id.as_ref())
                .map_or_else(|| saved.rotation(), CrownFunction::rotation);
            function.set_rotation(rotation);
            Ok(function)
        })
        .collect::<Result<Vec<_>, CrownError>>()?;
    let mut profile = saved.clone();
    profile.replace_functions(functions)?;
    Ok(profile)
}

pub(crate) fn motion_for(
    motion: openlogi_hid::crown::CrownMotion,
    amount: f64,
    ratcheted: bool,
) -> openlogi_sdk::ControlInput {
    use openlogi_hid::crown::RotationState;
    use openlogi_sdk::{ControlInput, Rotation, RotationPhase, TactileMode};
    ControlInput::Rotation(Rotation {
        amount,
        raw: i64::from(motion.slots),
        detents: i64::from(motion.ratchets),
        resolution: motion.resolution,
        tactile: if ratcheted {
            TactileMode::Ratchet
        } else {
            TactileMode::Smooth
        },
        phase: match motion.phase {
            RotationState::Start => RotationPhase::Start,
            RotationState::Active => RotationPhase::Move,
            RotationState::Stop => RotationPhase::Stop,
            RotationState::Inactive => RotationPhase::Inactive,
            other => RotationPhase::Unknown(u8::from(other)),
        },
        pressed: motion.pressed,
        touched: motion.touched,
        speed: i32::from(motion.speed),
    })
}
