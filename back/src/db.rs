use futures::io;
use log::debug;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    fs::TryLockError,
    io::Seek,
    path::{Path, PathBuf},
    time::Duration,
};
use web_push::{SubscriptionInfo, SubscriptionKeys};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Floor(pub u32);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SubscriptionId {
    pub endpoint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionRest {
    pub keys: SubscriptionKeys,
    pub floors: HashSet<Floor>,
}

struct ExclusiveAccessFile {
    file: std::fs::File,
}

impl ExclusiveAccessFile {
    /// Given that the file is always opened through `ExclusiveAccessFile`, then this opens the
    /// target file with exclusive access (only one `ExclusiveAccessFile` instance will have it open at a time)
    ///
    /// On Windows, this only works if the file is opened with one of `.read(true)`, `.read(true).append(true)`, or `.write(true)`.
    /// Files opened in append-only mode are not locked.
    ///
    /// However, this is intended to be used on Linux only, where, according to the manual of `flock`,
    /// "A shared or exclusive lock can be placed on a file regardless of
    /// the mode in which the file was opened."
    ///
    /// The future returned will wait for the file to be accessible for exclusive access.
    ///
    ///  ### Parameters:
    /// - path: The path to the file that should be opened with exclusive access
    pub async fn new(
        path: impl AsRef<Path>,
        options: std::fs::OpenOptions,
    ) -> Result<Self, io::Error> {
        let path = path.as_ref();
        let file = match options.open(path) {
            Err(e) => {
                return Err(e);
            }
            Ok(f) => f,
        };
        'lock_loop: loop {
            // The lock is release when the file is closed, so no need to explicitly unlock
            match file.try_lock() {
                Err(TryLockError::WouldBlock) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue 'lock_loop;
                }
                Err(e) => {
                    return Err(io::Error::other(e));
                }
                Ok(()) => break 'lock_loop,
            }
        }
        Ok(Self { file: file })
    }
}

type Subscriptions = HashMap<SubscriptionId, SubscriptionRest>;

/// The value is `true` if the floor has banana, and `false` otherwise
pub type BananaState = HashMap<Floor, bool>;

#[derive(Debug, Default)]
pub struct DatabaseOptions {
    /// A prefix for the filenames to which the database is saved. Shall not contain a path separator.
    pub db_file_prefix: Option<String>,
}

const DB_FILE_POSTFIX: &str = ".db.msgpack";

pub struct Database {
    subscription_db_file_path: PathBuf,
    banana_state_db_file_path: PathBuf,
}

enum DbOperationResult {
    WritebackNeeded,
    OnlyRead,
}

impl Database {
    pub fn new(options: DatabaseOptions) -> Self {
        let db_file_prefix = options.db_file_prefix.unwrap_or(String::new());
        let subscription_db_filename =
            format!("{}subscription{}", &db_file_prefix, DB_FILE_POSTFIX);
        let banana_state_db_filename =
            format!("{}banana_state{}", &db_file_prefix, DB_FILE_POSTFIX);
        Self {
            subscription_db_file_path: Path::new("./").join(subscription_db_filename),
            banana_state_db_file_path: Path::new("./").join(banana_state_db_filename),
        }
    }

    async fn access_db<DbType>(
        &self,
        db_path: &Path,
        operation: impl FnOnce(&mut DbType) -> DbOperationResult,
    ) -> Result<DbType, io::Error>
    where
        DbType: Serialize + DeserializeOwned + Default,
    {
        let mut open_options = std::fs::OpenOptions::new();
        open_options.read(true).write(true).create(true);
        let mut db_file = ExclusiveAccessFile::new(db_path, open_options).await?;

        let file_size = db_file.file.metadata().map(|m| m.len()).unwrap_or(0);
        if file_size == 0 {
            let mut serializer = rmp_serde::Serializer::new(&mut db_file.file);
            DbType::default()
                .serialize(&mut serializer)
                .map_err(|e| io::Error::other(e))?;
            db_file
                .file
                .seek(std::io::SeekFrom::Start(0))
                .map_err(|e| io::Error::other(e))?;
        }

        // parse file
        let mut db_contents: DbType =
            rmp_serde::from_read(&mut db_file.file).map_err(|e| io::Error::other(e))?;

        let operation_result = (operation)(&mut db_contents);
        let write_needed = matches!(operation_result, DbOperationResult::WritebackNeeded);

        if write_needed {
            db_file
                .file
                .seek(std::io::SeekFrom::Start(0))
                .map_err(|e| {
                    io::Error::other(format!(
                        "failed to `seek` while adding subscription, error: {}",
                        e
                    ))
                })?;
            db_file.file.set_len(0).map_err(|e| {
                io::Error::other(format!(
                    "failed to `set_len` while adding subscription, error: {}",
                    e
                ))
            })?;

            let mut serializer = rmp_serde::Serializer::new(db_file.file);
            db_contents
                .serialize(&mut serializer)
                .map_err(|e| io::Error::other(e))?;
        }

        Ok(db_contents)
    }

    pub async fn add_subscription(
        &self,
        subscription_info: SubscriptionInfo,
        floor: Floor,
    ) -> Result<(), io::Error> {
        debug!(
            "Adding subscription. New subscription: {:?}",
            subscription_info
        );

        self.access_db(
            &self.subscription_db_file_path,
            |subscriptions: &mut Subscriptions| {
                let subscription_id = SubscriptionId {
                    endpoint: subscription_info.endpoint,
                };
                subscriptions
                    .entry(subscription_id)
                    .or_insert_with(|| SubscriptionRest {
                        keys: subscription_info.keys,
                        floors: HashSet::new(),
                    })
                    .floors
                    .insert(floor);
                DbOperationResult::WritebackNeeded
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn remove_subscription(
        &self,
        subscription: &SubscriptionId,
        floor: Floor,
    ) -> Result<(), io::Error> {
        self.access_db(
            &self.subscription_db_file_path,
            |subscriptions: &mut Subscriptions| {
                if let Some(subscription) = subscriptions.get_mut(subscription) {
                    subscription.floors.remove(&floor);
                }
                DbOperationResult::WritebackNeeded
            },
        )
        .await
        .map(|_| ())
    }

    /// Returns subscriptions that are subscribed to the given floor
    pub async fn get_subscriptions(
        &self,
        floor: Floor,
    ) -> Result<HashSet<SubscriptionInfo>, io::Error> {
        let subscriptions = self
            .access_db::<Subscriptions>(&self.subscription_db_file_path, |_| {
                DbOperationResult::OnlyRead
            })
            .await?;

        Ok(subscriptions
            .iter()
            .filter_map(|(subscription_id, subscription_rest)| {
                if subscription_rest.floors.contains(&floor) {
                    Some(SubscriptionInfo {
                        endpoint: subscription_id.endpoint.clone(),
                        keys: subscription_rest.keys.clone(),
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    pub async fn get_floors_for_subscription(
        &self,
        subscription: &SubscriptionId,
    ) -> Result<HashSet<Floor>, io::Error> {
        let subscriptions = self
            .access_db::<Subscriptions>(&self.subscription_db_file_path, |_| {
                DbOperationResult::OnlyRead
            })
            .await?;

        Ok(subscriptions
            .get(subscription)
            .map(|subscription_rest| subscription_rest.floors.clone())
            .unwrap_or_default())
    }

    pub async fn get_banana_states(&self) -> Result<BananaState, io::Error> {
        let state = self
            .access_db::<BananaState>(&self.banana_state_db_file_path, |_| {
                DbOperationResult::OnlyRead
            })
            .await?;

        Ok(state)
    }

    pub async fn set_banana_state_for_floor(
        &self,
        floor: Floor,
        has_banana: bool,
    ) -> Result<(), io::Error> {
        self.access_db(
            &self.banana_state_db_file_path,
            |banana_state: &mut BananaState| {
                banana_state.insert(floor, has_banana);
                DbOperationResult::WritebackNeeded
            },
        )
        .await?;

        Ok(())
    }
}

mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use std::time::Instant;

    #[tokio::test]
    async fn test_exclusive_file_access_prevents_concurrent_access() {
        let test_file = "./test_exclusive_access.tmp";

        // Clean up before test
        let _ = std::fs::remove_file(test_file);

        // Create the test file
        std::fs::File::create(test_file).expect("Failed to create test file");

        let start = Instant::now();

        // Spawn two tasks trying to access the same file
        let task1 = {
            let test_file = test_file.to_string();
            tokio::spawn(async move {
                let mut options = std::fs::OpenOptions::new();
                options.read(true).write(true);
                let _lock = ExclusiveAccessFile::new(&test_file, options)
                    .await
                    .expect("Task 1 failed to acquire lock");

                // Hold the lock for 500ms
                tokio::time::sleep(Duration::from_millis(500)).await;
                Instant::now()
            })
        };

        let task2 = {
            let test_file = test_file.to_string();
            tokio::spawn(async move {
                // Give task1 time to acquire the lock first
                tokio::time::sleep(Duration::from_millis(50)).await;

                let mut options = std::fs::OpenOptions::new();
                options.read(true).write(true);
                let _lock = ExclusiveAccessFile::new(&test_file, options)
                    .await
                    .expect("Task 2 failed to acquire lock");

                Instant::now()
            })
        };

        let task1_released = task1.await.expect("Task 1 panicked");
        let task2_acquired = task2.await.expect("Task 2 panicked");

        let elapsed = start.elapsed();

        // Task 2 should have acquired the lock significantly after task 1 released it
        // This verifies that task 2 was blocked waiting for the lock
        assert!(
            task2_acquired >= task1_released,
            "Task 2 should have acquired lock after Task 1 released it"
        );

        // Total time should be roughly 500ms (task1) + retry time (task2), not concurrent
        assert!(
            elapsed.as_millis() >= 500,
            "Tasks should not run concurrently; elapsed: {:?}",
            elapsed
        );

        // Clean up
        let _ = std::fs::remove_file(test_file);
    }

    #[tokio::test]
    async fn test_exclusive_file_access_sequential_operations() {
        let test_file = "./test_sequential_access.tmp";

        // Clean up before test
        let _ = std::fs::remove_file(test_file);

        // Create the test file with initial content
        std::fs::write(test_file, "initial").expect("Failed to create test file");

        // First operation: read and verify initial content
        {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true);
            let _lock = ExclusiveAccessFile::new(test_file, options)
                .await
                .expect("First lock acquisition failed");
            // Lock is held and then released here
        }

        // Second operation: verify we can acquire the lock again
        {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true);
            let _lock = ExclusiveAccessFile::new(test_file, options)
                .await
                .expect("Second lock acquisition failed");
            // If we reach here, sequential access works correctly
        }

        // Clean up
        let _ = std::fs::remove_file(test_file);
    }

    #[tokio::test]
    async fn test_database_add_and_retrieve_subscription() {
        let db_file_prefix = "test_database_add_and_retrieve_subscription.";
        // Clean up database file before test

        let db_options = DatabaseOptions {
            db_file_prefix: Some(db_file_prefix.into()),
        };
        let db = Database::new(db_options);

        let _ = std::fs::remove_file(&db.subscription_db_file_path);

        // Create a test subscription
        let subscription_info = SubscriptionInfo {
            endpoint: "https://example.com/push/subscription1".to_string(),
            keys: SubscriptionKeys {
                auth: "test_auth_key".to_string(),
                p256dh: "test_p256dh_key".to_string(),
            },
        };
        let floor = Floor(2);

        // Add subscription
        db.add_subscription(subscription_info.clone(), floor)
            .await
            .expect("Failed to add subscription");

        // Retrieve subscriptions for the floor
        let retrieved = db
            .get_subscriptions(floor)
            .await
            .expect("Failed to retrieve subscriptions");

        // Verify the subscription is there
        assert_eq!(retrieved.len(), 1, "Should have exactly one subscription");
        let retrieved_sub = retrieved.iter().next().expect("No subscription found");
        assert_eq!(retrieved_sub.endpoint, subscription_info.endpoint);
        assert_eq!(retrieved_sub.keys.auth, subscription_info.keys.auth);
        assert_eq!(retrieved_sub.keys.p256dh, subscription_info.keys.p256dh);

        // Clean up
        let _ = std::fs::remove_file(&db.subscription_db_file_path);
    }

    #[tokio::test]
    async fn test_database_multiple_subscriptions_multiple_floors() {
        let db_file_prefix = "test_database_multiple_subscriptions_multiple_floors.";

        let db_options = DatabaseOptions {
            db_file_prefix: Some(db_file_prefix.into()),
        };
        let db = Database::new(db_options);

        // Clean up database file before test
        let _ = std::fs::remove_file(&db.subscription_db_file_path);

        // Create test subscriptions
        let subscription_info = SubscriptionInfo {
            endpoint: "https://example.com/push/subscription1".to_string(),
            keys: SubscriptionKeys {
                auth: "test_auth_key".to_string(),
                p256dh: "test_p256dh_key".to_string(),
            },
        };

        let floor_1 = Floor(1);
        let floor_2 = Floor(2);
        let floor_3 = Floor(3);

        // Add the same subscription to multiple floors
        db.add_subscription(subscription_info.clone(), floor_1)
            .await
            .expect("Failed to add subscription to floor 1");

        db.add_subscription(subscription_info.clone(), floor_2)
            .await
            .expect("Failed to add subscription to floor 2");

        db.add_subscription(subscription_info.clone(), floor_3)
            .await
            .expect("Failed to add subscription to floor 3");

        // Verify subscriptions exist on all floors
        let retrieved_floor_1 = db
            .get_subscriptions(floor_1)
            .await
            .expect("Failed to retrieve subscriptions for floor 1");
        assert_eq!(
            retrieved_floor_1.len(),
            1,
            "Floor 1 should have one subscription"
        );

        let retrieved_floor_2 = db
            .get_subscriptions(floor_2)
            .await
            .expect("Failed to retrieve subscriptions for floor 2");
        assert_eq!(
            retrieved_floor_2.len(),
            1,
            "Floor 2 should have one subscription"
        );

        let retrieved_floor_3 = db
            .get_subscriptions(floor_3)
            .await
            .expect("Failed to retrieve subscriptions for floor 3");
        assert_eq!(
            retrieved_floor_3.len(),
            1,
            "Floor 3 should have one subscription"
        );

        // Verify we can get all floors for the subscription
        let subscription_id = SubscriptionId {
            endpoint: subscription_info.endpoint.clone(),
        };
        let retrieved_floors = db
            .get_floors_for_subscription(&subscription_id)
            .await
            .expect("Failed to retrieve floors for subscription");

        assert_eq!(
            retrieved_floors.len(),
            3,
            "Subscription should be on 3 floors"
        );
        assert!(retrieved_floors.contains(&floor_1));
        assert!(retrieved_floors.contains(&floor_2));
        assert!(retrieved_floors.contains(&floor_3));

        // Clean up
        let _ = std::fs::remove_file(&db.subscription_db_file_path);
    }

    #[tokio::test]
    async fn test_banana_state_set_and_get_single_floor() {
        let db_file_prefix = "test_banana_state_set_and_get_single_floor.";

        let db_options = DatabaseOptions {
            db_file_prefix: Some(db_file_prefix.into()),
        };
        let db = Database::new(db_options);

        // Clean up database file before test
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);

        let floor = Floor(5);

        // Set banana state to true for the floor
        db.set_banana_state_for_floor(floor, true)
            .await
            .expect("Failed to set banana state to true");

        // Retrieve all banana states
        let states = db
            .get_banana_states()
            .await
            .expect("Failed to get banana states");

        // Verify the banana state was set correctly
        assert_eq!(
            states.len(),
            1,
            "Should have exactly one floor with banana state"
        );
        assert_eq!(
            states.get(&floor),
            Some(&true),
            "Floor 5 should have banana (true)"
        );

        // Update the banana state to false
        db.set_banana_state_for_floor(floor, false)
            .await
            .expect("Failed to set banana state to false");

        // Retrieve and verify the updated state
        let updated_states = db
            .get_banana_states()
            .await
            .expect("Failed to get updated banana states");

        assert_eq!(
            updated_states.get(&floor),
            Some(&false),
            "Floor 5 should not have banana (false)"
        );

        // Clean up
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);
    }

    #[tokio::test]
    async fn test_banana_state_multiple_floors() {
        let db_file_prefix = "test_banana_state_multiple_floors.";

        let db_options = DatabaseOptions {
            db_file_prefix: Some(db_file_prefix.into()),
        };
        let db = Database::new(db_options);

        // Clean up database file before test
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);

        let floor_1 = Floor(1);
        let floor_2 = Floor(2);
        let floor_3 = Floor(3);

        // Set banana states for multiple floors
        db.set_banana_state_for_floor(floor_1, true)
            .await
            .expect("Failed to set banana state for floor 1");

        db.set_banana_state_for_floor(floor_2, false)
            .await
            .expect("Failed to set banana state for floor 2");

        db.set_banana_state_for_floor(floor_3, true)
            .await
            .expect("Failed to set banana state for floor 3");

        // Retrieve all banana states
        let states = db
            .get_banana_states()
            .await
            .expect("Failed to get banana states");

        // Verify all states are correct
        assert_eq!(
            states.len(),
            3,
            "Should have exactly three floors with banana states"
        );
        assert_eq!(
            states.get(&floor_1),
            Some(&true),
            "Floor 1 should have banana (true)"
        );
        assert_eq!(
            states.get(&floor_2),
            Some(&false),
            "Floor 2 should not have banana (false)"
        );
        assert_eq!(
            states.get(&floor_3),
            Some(&true),
            "Floor 3 should have banana (true)"
        );

        // Update floor_2 to have banana
        db.set_banana_state_for_floor(floor_2, true)
            .await
            .expect("Failed to update banana state for floor 2");

        // Retrieve and verify all states are updated correctly
        let updated_states = db
            .get_banana_states()
            .await
            .expect("Failed to get updated banana states");

        assert_eq!(
            updated_states.get(&floor_1),
            Some(&true),
            "Floor 1 should still have banana"
        );
        assert_eq!(
            updated_states.get(&floor_2),
            Some(&true),
            "Floor 2 should now have banana"
        );
        assert_eq!(
            updated_states.get(&floor_3),
            Some(&true),
            "Floor 3 should still have banana"
        );

        // Clean up
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);
    }

    #[tokio::test]
    async fn test_banana_state_does_not_corrupt_subscriptions() {
        let db_file_prefix = "test_banana_state_does_not_corrupt_subscriptions.";

        let db_options = DatabaseOptions {
            db_file_prefix: Some(db_file_prefix.into()),
        };
        let db = Database::new(db_options);

        // Clean up database files before test
        let _ = std::fs::remove_file(&db.subscription_db_file_path);
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);

        // Add subscriptions
        let subscription_info_1 = SubscriptionInfo {
            endpoint: "https://example.com/push/subscription1".to_string(),
            keys: SubscriptionKeys {
                auth: "auth_key_1".to_string(),
                p256dh: "p256dh_key_1".to_string(),
            },
        };
        let subscription_info_2 = SubscriptionInfo {
            endpoint: "https://example.com/push/subscription2".to_string(),
            keys: SubscriptionKeys {
                auth: "auth_key_2".to_string(),
                p256dh: "p256dh_key_2".to_string(),
            },
        };

        let floor_1 = Floor(1);
        let floor_2 = Floor(2);

        db.add_subscription(subscription_info_1.clone(), floor_1)
            .await
            .expect("Failed to add subscription 1");
        db.add_subscription(subscription_info_2.clone(), floor_2)
            .await
            .expect("Failed to add subscription 2");

        // Now modify banana state multiple times
        db.set_banana_state_for_floor(floor_1, true)
            .await
            .expect("Failed to set banana state");
        db.set_banana_state_for_floor(floor_2, false)
            .await
            .expect("Failed to set banana state");
        db.set_banana_state_for_floor(floor_1, false)
            .await
            .expect("Failed to update banana state");

        // Verify subscriptions are still intact
        let retrieved_subs_floor_1 = db
            .get_subscriptions(floor_1)
            .await
            .expect("Failed to retrieve subscriptions for floor 1");
        let retrieved_subs_floor_2 = db
            .get_subscriptions(floor_2)
            .await
            .expect("Failed to retrieve subscriptions for floor 2");

        assert_eq!(
            retrieved_subs_floor_1.len(),
            1,
            "Floor 1 should still have one subscription"
        );
        assert_eq!(
            retrieved_subs_floor_2.len(),
            1,
            "Floor 2 should still have one subscription"
        );

        let sub_1 = retrieved_subs_floor_1
            .iter()
            .next()
            .expect("No subscription found");
        let sub_2 = retrieved_subs_floor_2
            .iter()
            .next()
            .expect("No subscription found");

        assert_eq!(
            sub_1.endpoint, subscription_info_1.endpoint,
            "Subscription 1 endpoint corrupted"
        );
        assert_eq!(
            sub_1.keys.auth, subscription_info_1.keys.auth,
            "Subscription 1 auth corrupted"
        );
        assert_eq!(
            sub_2.endpoint, subscription_info_2.endpoint,
            "Subscription 2 endpoint corrupted"
        );
        assert_eq!(
            sub_2.keys.auth, subscription_info_2.keys.auth,
            "Subscription 2 auth corrupted"
        );

        // Clean up
        let _ = std::fs::remove_file(&db.subscription_db_file_path);
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);
    }

    #[tokio::test]
    async fn test_subscription_changes_do_not_corrupt_banana_state() {
        let db_file_prefix = "test_subscription_changes_do_not_corrupt_banana_state.";

        let db_options = DatabaseOptions {
            db_file_prefix: Some(db_file_prefix.into()),
        };
        let db = Database::new(db_options);

        // Clean up database files before test
        let _ = std::fs::remove_file(&db.subscription_db_file_path);
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);

        // Set banana states
        let floor_1 = Floor(1);
        let floor_2 = Floor(2);
        let floor_3 = Floor(3);

        db.set_banana_state_for_floor(floor_1, true)
            .await
            .expect("Failed to set banana state for floor 1");
        db.set_banana_state_for_floor(floor_2, false)
            .await
            .expect("Failed to set banana state for floor 2");
        db.set_banana_state_for_floor(floor_3, true)
            .await
            .expect("Failed to set banana state for floor 3");

        // Now add and remove subscriptions
        let subscription_info = SubscriptionInfo {
            endpoint: "https://example.com/push/subscription1".to_string(),
            keys: SubscriptionKeys {
                auth: "test_auth".to_string(),
                p256dh: "test_p256dh".to_string(),
            },
        };

        db.add_subscription(subscription_info.clone(), floor_1)
            .await
            .expect("Failed to add subscription");
        db.add_subscription(subscription_info.clone(), floor_2)
            .await
            .expect("Failed to add subscription to floor 2");

        // Remove subscription from floor 1
        let subscription_id = SubscriptionId {
            endpoint: subscription_info.endpoint.clone(),
        };
        db.remove_subscription(&subscription_id, floor_1)
            .await
            .expect("Failed to remove subscription");

        // Verify banana states are still intact
        let states = db
            .get_banana_states()
            .await
            .expect("Failed to get banana states");

        assert_eq!(
            states.len(),
            3,
            "Should still have three floors with banana states"
        );
        assert_eq!(
            states.get(&floor_1),
            Some(&true),
            "Floor 1 banana state corrupted"
        );
        assert_eq!(
            states.get(&floor_2),
            Some(&false),
            "Floor 2 banana state corrupted"
        );
        assert_eq!(
            states.get(&floor_3),
            Some(&true),
            "Floor 3 banana state corrupted"
        );

        // Clean up
        let _ = std::fs::remove_file(&db.subscription_db_file_path);
        let _ = std::fs::remove_file(&db.banana_state_db_file_path);
    }
}
