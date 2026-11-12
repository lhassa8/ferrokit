use std::rc::Rc;
use ferrokit::*;

#[derive(Clone)]
struct Order {
    id: u32,
    customer: String,
    total: f64,
    paid: bool,
}

fn sample(n: u32) -> Vec<Order> {
    let names = ["Ada", "Grace", "Linus", "Margaret", "Ken"];
    (1..=n)
        .map(|id| Order {
            id,
            customer: names[id as usize % names.len()].to_string(),
            total: f64::from(id % 9_973) / 10.0,
            paid: id % 3 != 0,
        })
        .collect()
}

fn main() {
    let all = Rc::new(sample(1_000_000));
    let query = Signal::new(String::new());

    // Created once; the columns read from your own row type.
    let model = Rc::new(TableModel::new(
        ViewId(1),
        vec![
            Column::number("id", "Order", |o: &Order| f64::from(o.id)).decimals(0),
            Column::text("customer", "Customer", |o: &Order| o.customer.clone()).width(200.0),
            Column::number("total", "Total", |o: &Order| o.total).decimals(2),
            Column::check("paid", "Paid", |o: &Order| o.paid),
        ],
    ));
    let m = model.clone();

    App::new("Orders")
        .window(Window::new("Orders").size(640.0, 480.0).view(
            vstack((
                search_field(&query).accessibility_label("Search customers"),
                table(&model)
                    // Row numbers are positions in what you gave set_rows,
                    // whatever the user sorted by.
                    .on_activate(|row| println!("opened model row {row}"))
                    .accessibility_label("Orders"),
                // Runs again whenever `query` changes and updates the
                // table in place.
                watch(move || {
                    let q = query.get().to_lowercase();
                    let rows = all
                        .iter()
                        .filter(|o| q.is_empty() || o.customer.to_lowercase().contains(&q))
                        .cloned()
                        .collect();
                    m.set_rows(rows);
                }),
            ))
            .padding(12.0),
        ))
        .run();
}
