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
                            && connection == &entry.connection
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
        if entry.connection != *connection {
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
                if entry.connection != *connection {
                    *client = Client::new(connection.clone());
                    entry.connection = connection.clone();
                }
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
