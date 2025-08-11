// @generated automatically by Diesel CLI.

diesel::table! {
    auth_keys (id) {
        id -> Int4,
        user_id -> Int4,
        key -> Text,
        expiration -> Timestamp,
    }
}

diesel::table! {
    tasks (id) {
        id -> Int4,
        owner -> Nullable<Int4>,
        title -> Nullable<Text>,
        description -> Nullable<Text>,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        #[max_length = 255]
        username -> Varchar,
        #[max_length = 255]
        password -> Varchar,
    }
}

diesel::joinable!(auth_keys -> users (user_id));
diesel::joinable!(tasks -> users (owner));

diesel::allow_tables_to_appear_in_same_query!(
    auth_keys,
    tasks,
    users,
);
