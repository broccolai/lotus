pub fn destination_index(
    item_count: usize,
    source_index: usize,
    target_index: usize,
    insert_after: bool,
) -> Option<usize> {
    if item_count == 0 || source_index >= item_count || target_index >= item_count {
        return None;
    }

    let mut destination = target_index + usize::from(insert_after);
    if source_index < destination {
        destination -= 1;
    }

    Some(destination.min(item_count - 1))
}
