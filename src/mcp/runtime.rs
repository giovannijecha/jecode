//! Reuse each server's negotiated session across discovery and tool calls.
use super::{Client, Connection, Error, ErrorKind};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

struct Entry {
    connection: Connection,
    client: Arc<Mutex<Client>>,
}

fn same_session(a: &Connection, b: &Connection) -> bool {
    let mut a = a.clone();
    a.pending_scope.clone_from(&b.pending_scope);
    a == *b
}

#[derive(Default)]
pub struct Runtime {
    clients: Mutex<HashMap<(PathBuf, String), Entry>>,
}

impl Runtime {
    pub(super) fn prune(&self, home: &Path, connections: &[Connection]) -> Result<(), Error> {
        self.clients
            .lock()
            .map_err(|_| poisoned())?
            .retain(|(owner, id), entry| {
                owner != home
                    || connections.iter().any(|connection| {
                        &connection.id == id
                            && connection.enabled
                            && same_session(connection, &entry.connection)
                    })
            });
        Ok(())
    }

    pub(super) fn client(
        &self,
        home: &Path,
        connection: &Connection,
    ) -> Result<Arc<Mutex<Client>>, Error> {
        let mut clients = self.clients.lock().map_err(|_| poisoned())?;
        let entry = clients
            .entry((home.to_path_buf(), connection.id.clone()))
            .or_insert_with(|| Entry {
                connection: connection.clone(),
                client: Arc::new(Mutex::new(Client::new(connection.clone()))),
            });
        if !same_session(&entry.connection, connection) {
            *entry = Entry {
                connection: connection.clone(),
                client: Arc::new(Mutex::new(Client::new(connection.clone()))),
            };
        }
        Ok(Arc::clone(&entry.client))
    }

    /// Binds a locked client to the current record; a changed record, such as
    /// renewed tokens, starts a new server session in place.
    pub(super) fn bind(
        &self,
        home: &Path,
        shared: &Arc<Mutex<Client>>,
        client: &mut Client,
        connection: &Connection,
    ) -> Result<(), Error> {
        let mut clients = self.clients.lock().map_err(|_| poisoned())?;
        match clients.get_mut(&(home.to_path_buf(), connection.id.clone())) {
            Some(entry) if Arc::ptr_eq(&entry.client, shared) => {
                if !same_session(&entry.connection, connection) {
                    *client = Client::new(connection.clone());
                }
                entry.connection = connection.clone();
            }
            _ => *client = Client::new(connection.clone()),
        }
        Ok(())
    }
}

pub(super) fn poisoned() -> Error {
    Error::new(
        ErrorKind::Configuration,
        "MCP connection state is unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn pending_scope_does_not_replace_the_negotiated_client() {
        let home = Directory::new();
        let runtime = Runtime::default();
        let mut connection = Connection::draft("Scoped", "https://example.test/mcp");
        connection.id = "mcp-fixture".into();
        let first = runtime.client(home.path(), &connection).unwrap();
        connection.pending_scope = Some("deploy".into());
        runtime.prune(home.path(), &[connection.clone()]).unwrap();
        let second = runtime.client(home.path(), &connection).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let mut held = second.lock().unwrap();
        runtime
            .bind(home.path(), &second, &mut held, &connection)
            .unwrap();
        drop(held);
        connection.endpoint = "https://example.test/other".into();
        let changed = runtime.client(home.path(), &connection).unwrap();
        assert!(!Arc::ptr_eq(&second, &changed));
    }
}
