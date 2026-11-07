use crate::concept;

pub(super) struct Database {
    pub(super) id: concept::release::Template,
    pub(super) title: &'static str,
    pub(super) port: u16,
    pub(super) data_dir: &'static str,
    pub(super) password: &'static str,
    pub(super) env: &'static [(&'static str, &'static str)],
}

const POSTGRESQL: Database = Database {
    id: concept::release::Template::Postgresql18,
    title: "PostgreSQL",
    port: 5432,
    data_dir: "/var/lib/postgresql",
    password: "POSTGRES_PASSWORD",
    env: &[("POSTGRES_USER", "postgres"), ("POSTGRES_DB", "postgres")],
};

const MARIADB: Database = Database {
    id: concept::release::Template::Mariadb12,
    title: "MariaDB",
    port: 3306,
    data_dir: "/var/lib/mysql",
    password: "MARIADB_ROOT_PASSWORD",
    env: &[("MARIADB_DATABASE", "app")],
};

pub(super) const DEFAULT: &Database = &POSTGRESQL;
pub(super) const DATABASES: &[Database] = &[POSTGRESQL, MARIADB];
