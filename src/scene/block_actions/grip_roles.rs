//! Runtime grip eligibility for native key-point displacement bindings.
use super::*;

fn displacement(p: &Parameter, connections: &[BlockConnection]) -> u8 {
    if connections.len() != 2 || connections.iter().any(|connection| connection.code != p.id) {
        return 0;
    }
    match (
        p.point,
        p.xy,
        connections[0].name.as_str(),
        connections[1].name.as_str(),
    ) {
        (true, _, "XDelta", "YDelta") => 1,
        (false, _, "BaseXDelta", "BaseYDelta") => 1,
        (false, _, "EndXDelta", "EndYDelta") => 2,
        (false, true, "EndXDelta", "BaseYDelta") => 4,
        (false, true, "BaseXDelta", "EndYDelta") => 8,
        _ => 0,
    }
}

pub(crate) fn mask(doc: &CadDocument, definition: Handle, parameter: &Parameter) -> u8 {
    actions(doc, definition)
        .into_iter()
        .filter(|(_, _, kind, node)| *node == parameter.id && compatible(parameter, *kind))
        .fold(0, |mask, (handle, _, _, _)| {
            let Some(ObjectType::DynamicBlock(object)) = doc.objects.get(&handle) else {
                return mask;
            };
            mask | match &object.data {
                DynamicBlockData::MoveAction(action) => {
                    displacement(parameter, &action.connections)
                }
                DynamicBlockData::StretchAction(action) => {
                    displacement(parameter, &action.connections)
                }
                DynamicBlockData::PolarStretchAction(action) => {
                    match stretch::polar_key_point(action) {
                        Ok(XyKeyPoint::Base) => 1,
                        Ok(XyKeyPoint::End) => 2,
                        _ => 0,
                    }
                }
                // Non-displacement action families retain their existing role behavior.
                _ => 15,
            }
        })
}

#[cfg(test)]
mod tests;
