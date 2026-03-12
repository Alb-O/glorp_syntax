use super::*;

#[test]
fn cursor() {
	let mut locals = Locals::default();
	let scope1 = locals.push(ScopeData {
		definitions: Default::default(),
		range: 5..105,
		inherit: true,
		// NOTE: the subsequent call to `push` below will add scope2 to scope1's children.
		children: Default::default(),
		parent: Some(Scope::ROOT),
	});
	let scope2 = locals.push(ScopeData {
		definitions: Default::default(),
		range: 10..100,
		inherit: true,
		children: Default::default(),
		parent: Some(scope1),
	});

	let mut cursor = locals.scope_cursor(0);
	assert_eq!(cursor.current_scope(), Scope::ROOT);
	assert_eq!(cursor.advance(3), Scope::ROOT);
	assert_eq!(cursor.advance(5), scope1);
	assert_eq!(cursor.advance(8), scope1);
	assert_eq!(cursor.advance(10), scope2);
	assert_eq!(cursor.advance(50), scope2);
	assert_eq!(cursor.advance(100), scope1);
	assert_eq!(cursor.advance(105), Scope::ROOT);
	assert_eq!(cursor.advance(110), Scope::ROOT);

	let mut cursor = locals.scope_cursor(8);
	assert_eq!(cursor.current_scope(), scope1);
	assert_eq!(cursor.advance(10), scope2);
	assert_eq!(cursor.advance(100), scope1);
	assert_eq!(cursor.advance(110), Scope::ROOT);

	let mut cursor = locals.scope_cursor(10);
	assert_eq!(cursor.current_scope(), scope2);
	assert_eq!(cursor.advance(100), scope1);
	assert_eq!(cursor.advance(110), Scope::ROOT);
}
