//! Native Block Properties Table authoring.
use super::block_authoring::{self as authoring, *};
use codec::objects::{
    BlockElement, BlockEvalValue, BlockGrip, BlockGripExpression, BlockOnePointParameter,
    BlockParameter, BlockPropertiesTable, BlockPropertiesTableField, BlockPropertiesTableValue,
    DynamicBlockData, ObjectType,
};
use codec::{CadDocument, Handle};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Column {
    pub handle: Handle,
    pub node_id: i32,
    pub connection_name: String,
    pub display_name: String,
    pub type_code: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TableView {
    pub handle: Handle,
    pub table: BlockPropertiesTable,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<f64>>,
    pub row_start: usize,
    pub trailer_start: usize,
    pub must_match: bool,
}

pub(crate) type Grid = (Handle, Vec<Column>, Vec<Vec<String>>);

pub(crate) fn graph_handle(document: &CadDocument, definition: Handle) -> Option<Handle> {
    let xdict = document.extension_dictionary_handle(definition)?;
    let ObjectType::Dictionary(dictionary) = document.objects.get(&xdict)? else {
        return None;
    };
    dictionary.get("ACAD_ENHANCEDBLOCK")
}

pub(crate) fn table(
    document: &CadDocument,
    definition: Handle,
) -> Result<Option<TableView>, &'static str> {
    let graph = graph_handle(document, definition).ok_or("The authoring graph is unavailable.")?;
    let mut found = document.objects.values().filter_map(|object| match object {
        ObjectType::DynamicBlock(object) if object.owner == graph => match &object.data {
            DynamicBlockData::PropertiesTable(table) => Some((object.handle, table.clone())),
            _ => None,
        },
        _ => None,
    });
    let Some((handle, table)) = found.next() else {
        return Ok(None);
    };
    if found.next().is_some() {
        return Err("A block can contain only one Block Properties Table.");
    }
    let mut view = parse(handle, table)?;
    for column in &mut view.columns {
        column.node_id = super::block_actions::all_parameters(document, definition)
            .iter()
            .find(|parameter| parameter.handle == column.handle)
            .map(|parameter| parameter.id)
            .unwrap_or(0);
        if column.node_id != 0 {
            column.type_code = i64::from(column.node_id);
        }
        column.display_name = super::block_lookup::properties(document, definition)
            .into_iter()
            .find(|property| {
                property.column.node_id == column.node_id
                    && property
                        .column
                        .connection_name
                        .eq_ignore_ascii_case(&column.connection_name)
            })
            .map(|property| property.label)
            .unwrap_or_else(|| column.connection_name.clone());
    }
    Ok(Some(view))
}

fn integer(field: &BlockPropertiesTableField) -> Option<i64> {
    match field.value {
        BlockPropertiesTableValue::Integer(value) => Some(value),
        _ => None,
    }
}

fn real(field: &BlockPropertiesTableField) -> Option<f64> {
    match field.value {
        BlockPropertiesTableValue::Real(value) => Some(value),
        _ => None,
    }
}

fn handle(field: &BlockPropertiesTableField) -> Option<Handle> {
    match field.value {
        BlockPropertiesTableValue::Handle(value) => Some(value),
        _ => None,
    }
}

fn parse(table_handle: Handle, table: BlockPropertiesTable) -> Result<TableView, &'static str> {
    let fields = &table.fields;
    let column_count_index = fields
        .iter()
        .position(|field| field.code == 91)
        .ok_or("The Block Properties Table has no property columns.")?;
    let column_count = usize::try_from(
        integer(&fields[column_count_index])
            .ok_or("The Block Properties Table column count is invalid.")?,
    )
    .map_err(|_| "The Block Properties Table column count is invalid.")?;
    if column_count == 0 || column_count > 64 {
        return Err("The Block Properties Table column count is unsupported.");
    }
    let mut columns = Vec::with_capacity(column_count);
    let mut index = column_count_index + 1;
    for _ in 0..column_count {
        let column_handle = fields
            .get(index)
            .filter(|field| field.code == 340)
            .and_then(handle)
            .ok_or("A Block Properties Table property binding is invalid.")?;
        index += 1;
        let mut name = None;
        for field in fields.iter().skip(index) {
            if field.code == 301 && name.is_none() {
                if let BlockPropertiesTableValue::Text(value) = &field.value {
                    name = Some(value.clone());
                }
            } else if field.code == 90 && name.is_some() {
                break;
            }
        }
        let name = name.ok_or("A Block Properties Table property name is missing.")?;
        let end = fields
            .iter()
            .enumerate()
            .skip(index)
            .find(|(_, field)| field.code == 340)
            .map(|(i, _)| i)
            .ok_or("A Block Properties Table property record is incomplete.")?;
        let property_id = fields
            .iter()
            .enumerate()
            .skip(index)
            .take(end - index)
            .find(|(_, field)| field.code == 90)
            .and_then(|(_, field)| integer(field))
            .ok_or("A Block Properties Table property type is missing.")?;
        columns.push(Column {
            handle: column_handle,
            node_id: 0,
            connection_name: name.clone(),
            display_name: name,
            type_code: property_id,
        });
        index = end + 1;
    }
    let row_count_index = fields
        .iter()
        .enumerate()
        .skip(index)
        .find(|(_, field)| field.code == 92)
        .map(|(i, _)| i)
        .ok_or("The Block Properties Table row data is unavailable.")?;
    let row_count = usize::try_from(
        integer(&fields[row_count_index])
            .ok_or("The Block Properties Table row count is invalid.")?,
    )
    .map_err(|_| "The Block Properties Table row count is invalid.")?;
    if row_count == 0 || row_count > 1000 {
        return Err("The Block Properties Table row count is unsupported.");
    }
    let row_start = row_count_index + 1;
    let mut rows = Vec::with_capacity(row_count);
    index = row_start;
    for expected_row in 0..row_count {
        let row_index = fields
            .get(index)
            .filter(|field| field.code == 90)
            .and_then(integer)
            .ok_or("A Block Properties Table row index is invalid.")?;
        if row_index != expected_row as i64 {
            return Err("The Block Properties Table rows are not ordered.");
        }
        index += 1;
        let mut row = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            let value_type = fields
                .get(index)
                .filter(|field| field.code == 170)
                .and_then(integer)
                .ok_or("A Block Properties Table cell type is unsupported.")?;
            if value_type != 40 {
                return Err("Only numeric Block Properties Table cells are supported.");
            }
            let value = fields
                .get(index + 1)
                .filter(|field| field.code == 140)
                .and_then(real)
                .filter(|value| value.is_finite())
                .ok_or("A Block Properties Table cell value is invalid.")?;
            row.push(value);
            index += 2;
        }
        rows.push(row);
    }
    let trailer_start = fields
        .iter()
        .enumerate()
        .skip(index)
        .find(|(_, field)| field.code == 93)
        .map(|(i, _)| i)
        .ok_or("The Block Properties Table trailer is unavailable.")?;
    let must_match = fields
        .get(trailer_start + 1)
        .and_then(|f| match f.value {
            BlockPropertiesTableValue::Boolean(v) if f.code == 290 => Some(v),
            _ => None,
        })
        .unwrap_or(false);
    Ok(TableView {
        handle: table_handle,
        table,
        columns,
        rows,
        row_start,
        trailer_start,
        must_match,
    })
}

pub(crate) fn columns(document: &CadDocument, definition: Handle) -> Vec<Column> {
    let parameters = super::block_actions::all_parameters(document, definition);
    super::block_lookup::properties(document, definition)
        .into_iter()
        .filter_map(|property| {
            let parameter = parameters
                .iter()
                .find(|parameter| parameter.id == property.column.node_id)?;
            if !property
                .column
                .connection_name
                .eq_ignore_ascii_case("UpdatedDistance")
                && !property
                    .column
                    .connection_name
                    .eq_ignore_ascii_case("UpdatedAngle")
            {
                return None;
            }
            Some(Column {
                handle: parameter.handle,
                node_id: parameter.id,
                connection_name: property.column.connection_name,
                display_name: property.label,
                type_code: i64::from(parameter.id),
            })
        })
        .collect()
}

pub(crate) fn grid_table(
    document: &CadDocument,
    definition: Handle,
) -> Result<Option<Grid>, &'static str> {
    let Some(view) = table(document, definition)? else {
        return Ok(None);
    };
    let rows = view
        .rows
        .iter()
        .map(|row| row.iter().map(ToString::to_string).collect())
        .collect();
    Ok(Some((view.handle, view.columns, rows)))
}

pub(crate) fn apply(
    document: &mut CadDocument,
    definition: Handle,
    columns: &[Column],
    rows: &[Vec<f64>],
    must_match: bool,
) -> Result<Handle, &'static str> {
    if columns.is_empty() || columns.len() > 64 || rows.is_empty() || rows.len() > 1000 {
        return Err("Add at least one property and one complete row.");
    }
    if rows
        .iter()
        .any(|row| row.len() != columns.len() || row.iter().any(|value| !value.is_finite()))
    {
        return Err("Complete each numeric property cell with a finite value.");
    }
    let mut working = document.clone();
    graph_handle(&working, definition).ok_or("The authoring graph is unavailable.")?;
    let previous = table(&working, definition)?;
    let mut new_table_eval = None;
    let handle = if let Some(view) = &previous {
        view.handle
    } else {
        let (graph, slots) = authoring::prepare(&mut working, definition)
            .map_err(|_| "The block authoring graph cannot accept a Block Properties Table.")?;
        register_classes(
            &mut working,
            &[("BLOCKPROPERTIESTABLE", "AcDbBlockPropertiesTable", 1153)],
        );
        let handle = working.allocate_handle();
        let element = BlockElement {
            eval: expression(slots.eval, NO_VALUE, BlockEvalValue::None),
            name: "Block Table".into(),
            major: MAJOR,
            minor: MINOR,
            eed_1071: 0,
        };
        new_table_eval = Some(slots.eval);
        let parameter = BlockOnePointParameter {
            parameter: BlockParameter {
                element,
                show_properties: true,
                chain_actions: false,
            },
            property_count: slots.eval + 1,
            ..Default::default()
        };
        let table = BlockPropertiesTable {
            parameter,
            fields: vec![],
        };
        working.objects.insert(
            handle,
            ObjectType::DynamicBlock(dynamic(
                handle,
                graph,
                "BLOCKPROPERTIESTABLE",
                "AcDbBlockPropertiesTable",
                DynamicBlockData::PropertiesTable(table),
            )),
        );
        authoring::append_single(&mut working, graph, slots, handle);
        handle
    };
    let old_fields = previous.as_ref().map(|view| view.table.fields.as_slice());
    let new_fields = encode_fields(columns, rows, old_fields, must_match)?;
    let Some(ObjectType::DynamicBlock(object)) = working.objects.get_mut(&handle) else {
        return Err("The Block Properties Table object is unavailable.");
    };
    let DynamicBlockData::PropertiesTable(table) = &mut object.data else {
        return Err("The selected object is not a Block Properties Table.");
    };
    table.fields = new_fields;
    if let Some(eval) = new_table_eval {
        table.parameter.property_count = eval + 1;
    }
    let graph = graph_handle(&working, definition).ok_or("The authoring graph is unavailable.")?;
    ensure_grip(&mut working, definition, graph, handle)?;
    let _ = graph;
    *document = working;
    Ok(handle)
}

fn ensure_grip(
    document: &mut CadDocument,
    definition: Handle,
    graph: Handle,
    table_handle: Handle,
) -> Result<(), &'static str> {
    let already = document.objects.values().any(|value| matches!(value, ObjectType::DynamicBlock(object) if object.owner == graph && matches!(&object.data, DynamicBlockData::PropertiesTableGrip(_))));
    let Some(ObjectType::DynamicBlock(table_obj)) = document.objects.get(&table_handle) else {
        return Err("The Block Properties Table object is unavailable.");
    };
    let DynamicBlockData::PropertiesTable(table_data) = &table_obj.data else {
        return Err("The selected object is not a Block Properties Table.");
    };
    let location = table_data.parameter.definition_point;
    let table_eval = table_data.parameter.parameter.element.eval.node_id;
    if already {
        let grip_eval = document.objects.values().find_map(|value| match value {
            ObjectType::DynamicBlock(object) if object.owner == graph => match &object.data {
                DynamicBlockData::PropertiesTableGrip(grip) => Some(grip.element.eval.node_id),
                _ => None,
            },
            _ => None,
        });
        if let Some(grip_eval) = grip_eval {
            if let Some(ObjectType::DynamicBlock(object)) = document.objects.get_mut(&table_handle)
            {
                if let DynamicBlockData::PropertiesTable(table) = &mut object.data {
                    table.parameter.property_count = grip_eval;
                }
            }
        }
        return Ok(());
    }
    let (graph, slots) = authoring::prepare(document, definition)
        .map_err(|_| "The block authoring graph cannot accept a table grip.")?;
    authoring::register_classes(
        document,
        &[
            ("BLOCKPROPERTIESTABLEGRIP", "AcDbBlockGrip", 55),
            ("BLOCKGRIPLOCATIONCOMPONENT", "AcDbBlockGripExpr", 20),
        ],
    );
    let grip = document.allocate_handle();
    let x = document.allocate_handle();
    let y = document.allocate_handle();
    let grip_data = BlockGrip {
        element: BlockElement {
            eval: expression(slots.eval, NO_VALUE, BlockEvalValue::None),
            name: "Grip".into(),
            major: MAJOR,
            minor: MINOR,
            eed_1071: 0,
        },
        flags_91: slots.eval + 1,
        flags_92: slots.eval + 2,
        location,
        insert_cycling: false,
        insert_cycling_weight: -1,
    };
    document.objects.insert(
        grip,
        ObjectType::DynamicBlock(dynamic(
            grip,
            graph,
            "BLOCKPROPERTIESTABLEGRIP",
            "AcDbBlockGrip",
            DynamicBlockData::PropertiesTableGrip(grip_data),
        )),
    );
    for (handle, eval, axis) in [
        (x, slots.eval + 1, "UpdatedX"),
        (y, slots.eval + 2, "UpdatedY"),
    ] {
        document.objects.insert(
            handle,
            ObjectType::DynamicBlock(dynamic(
                handle,
                graph,
                "BLOCKGRIPLOCATIONCOMPONENT",
                "AcDbBlockGripExpr",
                DynamicBlockData::GripLocationComponent(BlockGripExpression {
                    eval: expression(eval, 40, BlockEvalValue::Real(0.0)),
                    grip_type: table_eval,
                    expression: axis.into(),
                }),
            )),
        );
    }
    authoring::append_components(document, graph, slots, &[grip, x, y], &[1, 2]);
    if let Some(ObjectType::DynamicBlock(object)) = document.objects.get_mut(&table_handle) {
        if let DynamicBlockData::PropertiesTable(table) = &mut object.data {
            table.parameter.property_count = slots.eval;
        }
    }
    Ok(())
}

fn encode_fields(
    columns: &[Column],
    rows: &[Vec<f64>],
    old: Option<&[BlockPropertiesTableField]>,
    must_match: bool,
) -> Result<Vec<BlockPropertiesTableField>, &'static str> {
    let mut fields = if let Some(fields) = old {
        fields.to_vec()
    } else {
        vec![int(90, 2), text(300, "Block Table1"), text(301, "")]
    };
    if old.is_none() {
        fields.push(int(91, columns.len() as i64));
        for column in columns {
            fields.extend([
                handle_field(340, column.handle),
                int(170, 0),
                int(171, -1),
                text(300, ""),
                text(301, &column.connection_name),
                int(90, column.node_id as i64),
                int(170, -9999),
                int(170, -9999),
                boolean(290, false),
                boolean(291, true),
                boolean(292, true),
                boolean(
                    293,
                    column.connection_name.eq_ignore_ascii_case("UpdatedAngle"),
                ),
                boolean(294, false),
                text(302, ""),
                handle_field(340, Handle::NULL),
            ]);
        }
    } else {
        let view = parse(
            Handle::NULL,
            BlockPropertiesTable {
                parameter: Default::default(),
                fields: fields.clone(),
            },
        )?;
        if view.columns.len() != columns.len()
            || view.columns.iter().zip(columns).any(|(old, new)| {
                old.handle != new.handle
                    || old.connection_name != new.connection_name
                    || (old.type_code != new.type_code
                        && old.node_id != 0
                        && old.node_id != new.node_id)
            })
        {
            return Err(
                "Changing Block Properties Table columns is not supported for an existing table.",
            );
        }
        fields.truncate(view.row_start - 1);
        for column in columns {
            if let Some(position) = fields.iter().enumerate().skip(4).find_map(|(i, f)|
                (f.code == 340 && matches!(f.value, BlockPropertiesTableValue::Handle(h) if h == column.handle)).then_some(i)) {
                let end = fields.iter().enumerate().skip(position + 1).find(|(_, f)| f.code == 340).map(|(i, _)| i).unwrap_or(fields.len());
                if let Some(field) = fields.iter_mut().take(end).skip(position + 1).find(|f| f.code == 90) {
                    field.value = BlockPropertiesTableValue::Integer(i64::from(column.node_id));
                }
                if let Some(field) = fields.iter_mut().take(end).skip(position + 1).find(|f| f.code == 293) {
                    field.value = BlockPropertiesTableValue::Boolean(column.connection_name.eq_ignore_ascii_case("UpdatedAngle"));
                }
            }
        }
    }
    fields.push(int(92, rows.len() as i64));
    for (row_index, row) in rows.iter().enumerate() {
        fields.push(int(90, row_index as i64));
        for value in row {
            fields.push(int(170, 40));
            fields.push(real_field(140, *value));
        }
    }
    fields.push(int(93, rows.len() as i64));
    fields.extend([
        boolean(290, must_match),
        boolean(291, true),
        boolean(292, false),
    ]);
    Ok(fields)
}

fn int(code: i32, value: i64) -> BlockPropertiesTableField {
    BlockPropertiesTableField {
        code,
        value: BlockPropertiesTableValue::Integer(value),
    }
}

fn real_field(code: i32, value: f64) -> BlockPropertiesTableField {
    BlockPropertiesTableField {
        code,
        value: BlockPropertiesTableValue::Real(value),
    }
}

fn text(code: i32, value: &str) -> BlockPropertiesTableField {
    BlockPropertiesTableField {
        code,
        value: BlockPropertiesTableValue::Text(value.into()),
    }
}

fn boolean(code: i32, value: bool) -> BlockPropertiesTableField {
    BlockPropertiesTableField {
        code,
        value: BlockPropertiesTableValue::Boolean(value),
    }
}

fn handle_field(code: i32, value: Handle) -> BlockPropertiesTableField {
    BlockPropertiesTableField {
        code,
        value: BlockPropertiesTableValue::Handle(value),
    }
}
