// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::data::{Conn, Db, DbConnection, QueryExecutor};
use crate::error::Error;

use super::checkpoint::{Checkpoint, CheckpointId};
use async_graphql::*;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, QueryResult};
use sui_indexer::schema::checkpoints;

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub(crate) struct AvailableRange {
    pub first: u64,
    pub last: u64,
}

/// Range of checkpoints that the RPC is guaranteed to produce a consistent response for.
#[Object]
impl AvailableRange {
    async fn first(&self, ctx: &Context<'_>) -> Result<Option<Checkpoint>> {
        Checkpoint::query(ctx, CheckpointId::by_seq_num(self.first), self.last)
            .await
            .extend()
    }

    async fn last(&self, ctx: &Context<'_>) -> Result<Option<Checkpoint>> {
        Checkpoint::query(ctx, CheckpointId::by_seq_num(self.last), self.last)
            .await
            .extend()
    }
}

impl AvailableRange {
    /// Look up the available range when viewing the data consistently at `checkpoint_viewed_at`.
    pub(crate) async fn query(db: &Db, checkpoint_viewed_at: u64) -> Result<Self, Error> {
        let available_range_len = db.limits.available_range_len;
        let Some(range): Option<Self> = db
            .execute(move |conn| Self::result(conn, checkpoint_viewed_at, available_range_len))
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch available range: {e}")))?
        else {
            return Err(Error::Client(format!(
                "Requesting data at checkpoint {checkpoint_viewed_at}, outside the available \
                 range.",
            )));
        };

        Ok(range)
    }

    /// Look up the available range when viewing the data consistently at `checkpoint_viewed_at`.
    /// Made available on the `Conn` type to make it easier to call as part of other queries.
    ///
    /// Returns an error if there was an issue querying the database, Ok(None) if the checkpoint
    /// being viewed is not in the database's available range, or Ok(Some(AvailableRange))
    /// otherwise.
    pub(crate) fn result(
        conn: &mut Conn,
        checkpoint_viewed_at: u64,
        available_range_len: u64,
    ) -> QueryResult<Option<Self>> {
        use checkpoints::dsl as checkpoints;

        let rhs: Option<i64> = conn
            .result(move || {
                let rhs = checkpoints::checkpoints
                    .select(checkpoints::sequence_number)
                    .order(checkpoints::sequence_number.desc())
                    .limit(1);
                rhs
            })
            .optional()?;

        let (first, mut last) = match rhs {
            Some(rhs) => ((rhs as u64).saturating_sub(available_range_len), rhs as u64),
            None => (0, 0),
        };

        if checkpoint_viewed_at < first || last < checkpoint_viewed_at {
            return Ok(None);
        }

        last = checkpoint_viewed_at;
        Ok(Some(Self { first, last }))
    }
}
