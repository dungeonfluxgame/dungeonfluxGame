//! How a sender is named in a report and an audit line: `engine`, or `seat:N`.

use df_table::envelope::Sender;

pub fn sender_label(sender: Sender) -> String {
    match sender {
        Sender::Engine => "engine".to_owned(),
        Sender::Seat(seat) => format!("seat:{}", seat.get()),
    }
}
