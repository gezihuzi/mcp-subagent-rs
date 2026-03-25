pub fn compute_invoice_total(
    units: i32,
    unit_price_cents: i64,
    discount_percent: i32,
) -> Result<i64, &'static str> {
    if units < 0 {
        return Err("units must be non-negative");
    }
    if unit_price_cents < 0 {
        return Err("unit price must be non-negative");
    }
    if !(0..=100).contains(&discount_percent) {
        return Err("discount_percent must be within 0..=100");
    }

    let subtotal = (units as i64) * unit_price_cents;
    let discount = subtotal * (discount_percent as i64) / 100;
    Ok(subtotal - discount)
}

#[cfg(test)]
mod tests {
    use super::compute_invoice_total;

    #[test]
    fn computes_total_with_discount() {
        let total = compute_invoice_total(10, 500, 20).expect("must be valid");
        assert_eq!(total, 4000);
    }

    #[test]
    fn rejects_invalid_discount() {
        let err = compute_invoice_total(1, 100, 200).expect_err("must reject invalid discount");
        assert_eq!(err, "discount_percent must be within 0..=100");
    }

    #[test]
    fn rejects_negative_units() {
        let err = compute_invoice_total(-1, 100, 0).expect_err("must reject invalid units");
        assert_eq!(err, "units must be non-negative");
    }
}
