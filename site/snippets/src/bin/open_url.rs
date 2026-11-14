use ferrokit::*;

fn main() {
    let opened = Signal::new(String::from("No link yet"));
    App::new("Notes")
        // Every notes://… link, whether it launched the app or came while it ran.
        .on_open_url(move |url| match url.strip_prefix("notes://open/") {
            Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
                opened.set(format!("Opening note {id}"));
            }
            // Anyone can send the app a link: ignore what you don't recognise.
            _ => opened.set(format!("Ignored {url}")),
        })
        .window(Window::new("Notes").size(360.0, 120.0).view(text(move || opened.get()).padding(20.0)))
        .run();
}
