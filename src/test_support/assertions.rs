pub fn array<T, const N: usize>(items: &[T]) -> &[T; N] {
    items.try_into().expect("expected collection cardinality")
}

pub fn only<T>(items: &[T]) -> &T {
    let [item] = array(items);
    item
}

#[cfg(feature = "remote")]
pub fn array_mut<T, const N: usize>(items: &mut [T]) -> &mut [T; N] {
    items.try_into().expect("expected collection cardinality")
}

#[cfg(feature = "remote")]
pub fn only_mut<T>(items: &mut [T]) -> &mut T {
    let [item] = array_mut(items);
    item
}
