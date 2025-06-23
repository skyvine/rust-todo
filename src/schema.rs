// @generated automatically by Diesel CLI.

diesel::table! {
    users (id) {
        id -> Int4,
        #[max_length = 80]
        username -> Varchar,
        #[max_length = 80]
        password -> Varchar,
    }
}
