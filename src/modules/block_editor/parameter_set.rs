//! Parameter Set shortcuts compose an existing parameter placement with its
//! corresponding action or table editor.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParameterSet {
    pub parameter: &'static str,
    pub follow_up: &'static str,
}

pub(crate) fn get(label: &str) -> Option<ParameterSet> {
    let value = match label {
        "Point Move" => ("POINT", "BACTION MOVE"),
        "Linear Move" => ("LINEAR", "BACTION MOVE"),
        "Linear Stretch" => ("LINEAR", "BACTION STRETCH"),
        "Linear Array" => ("LINEAR", "BACTION ARRAY"),
        "Polar Move" => ("POLAR", "BACTION MOVE"),
        "Polar Stretch" => ("POLAR", "BACTION POLAR STRETCH"),
        "Polar Array" => ("POLAR", "BACTION ARRAY"),
        "XY Move" => ("XY", "BACTION MOVE"),
        "XY Stretch Box Set" => ("XY", "BACTION STRETCH"),
        "XY Array Box Set" => ("XY", "BACTION ARRAY"),
        "Rotation Set" => ("ROTATION", "BACTION ROTATE"),
        "Flip Set" => ("FLIP", "BACTION FLIP"),
        "Visibility Set" => ("VISIBILITY", "BVSTATE DIALOG"),
        "Lookup Set" => ("LOOKUP", "BLOOKUPTABLE"),
        _ => return None,
    };
    Some(ParameterSet {
        parameter: value.0,
        follow_up: value.1,
    })
}

pub(crate) fn unavailable_reason(label: &str) -> Option<&'static str> {
    match label {
        "Linear Move Pair"
        | "Linear Stretch Pair"
        | "Polar Move Pair"
        | "Polar Stretch Pair"
        | "XY Move Pair" => Some("Paired parameter authoring is not available yet."),
        "XY Move Box Set" => Some("XY move box-set authoring is not available yet."),
        _ => None,
    }
}
