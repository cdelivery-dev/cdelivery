pub(super) struct Database {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) image: &'static str,
    pub(super) port: u16,
    pub(super) data_dir: &'static str,
    pub(super) password: &'static str,
    pub(super) env: &'static [(&'static str, &'static str)],
}

const POSTGRESQL: Database = Database {
    id: "postgresql",
    title: "PostgreSQL",
    image: "postgres:18",
    port: 5432,
    data_dir: "/var/lib/postgresql/data",
    password: "POSTGRES_PASSWORD",
    env: &[
        ("POSTGRES_USER", "postgres"),
        ("POSTGRES_DB", "postgres"),
        ("PGDATA", "/var/lib/postgresql/data"),
    ],
};

const MYSQL: Database = Database {
    id: "mysql",
    title: "MySQL",
    image: "mysql:9.7",
    port: 3306,
    data_dir: "/var/lib/mysql",
    password: "MYSQL_ROOT_PASSWORD",
    env: &[("MYSQL_DATABASE", "app")],
};

const MARIADB: Database = Database {
    id: "mariadb",
    title: "MariaDB",
    image: "mariadb:12.3",
    port: 3306,
    data_dir: "/var/lib/mysql",
    password: "MARIADB_ROOT_PASSWORD",
    env: &[("MARIADB_DATABASE", "app")],
};

pub(super) const DEFAULT: &Database = &POSTGRESQL;
pub(super) const DATABASES: &[Database] = &[POSTGRESQL, MYSQL, MARIADB];
