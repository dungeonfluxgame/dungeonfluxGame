use df_commerce::draft_image::Binding;
use df_finance::draft_image::{Error, Receipt};
use tokio_postgres::Transaction;

use super::Config;

fn failure(error: tokio_postgres::Error) -> Error {
    df_observe::record_audit_line(
        "IMAGERY",
        format!(
            "IMAGERY draft_admission status=failed sqlstate={}",
            error
                .as_db_error()
                .map(|db| db.code().code())
                .unwrap_or("none")
        ),
    );
    if error
        .as_db_error()
        .is_some_and(|db| matches!(db.code().code(), "23505" | "23503"))
    {
        Error::LinkConflict
    } else {
        Error::Finance(error.into())
    }
}

pub(super) async fn check_finance(
    tx: &Transaction<'_>,
    account: df_types::AccountId,
    b: &Binding,
    r: &Receipt,
) -> Result<(), Error> {
    let row=tx.query_opt("SELECT 1 FROM finance.provider_attempt a JOIN finance.reservation v USING(reservation_id) \
        JOIN finance.operation_receipt o ON o.namespace='draft_admission' AND o.principal=a.account_id \
        AND o.operation_id=v.operation_id WHERE a.attempt_id=$1 AND a.account_id=$2 AND a.campaign_id IS NULL \
        AND a.effect_id=$3 AND a.media_asset_id=$4 AND v.reservation_id=$5 AND v.account_id=$2 \
        AND v.session_id IS NULL AND o.fingerprint=$6 AND o.result_ref=v.reservation_id",
        &[&r.attempt.as_bytes().as_slice(),&account.as_bytes().as_slice(),&b.effect.as_slice(),
          &b.asset.0.as_slice(),&r.reservation.as_bytes().as_slice(),&r.fingerprint.as_slice()])
        .await.map_err(failure)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    Ok(())
}

pub(super) async fn check_catalog(tx: &Transaction<'_>, c: &Config) -> Result<(), Error> {
    let a = &c.admission;
    let row = tx
        .query_opt(
            "SELECT 1 FROM media.asset WHERE asset_id=$1 AND attempt=$2 \
        AND kind='image' AND provenance='generated' AND campaign_id IS NULL \
        AND charge_scope_kind='account' AND charge_scope_id=$3 AND origin_account=$3 \
        AND audience_kind='owner_private' AND audience_account=$3 \
        AND dedupe_scope=$4 AND request_key=$5 AND retention_class='personal' FOR UPDATE",
            &[
                &a.asset.as_bytes().as_slice(),
                &a.attempt.as_bytes().as_slice(),
                &a.origin_account.map(|id| id.as_bytes().to_vec()),
                &a.dedupe_scope.as_bytes().as_slice(),
                &c.recipe_key.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(failure)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    Ok(())
}

pub(super) async fn check_job(
    tx: &Transaction<'_>,
    c: &Config,
    scope: &[u8],
    b: &Binding,
    admitted_at_ms: i64,
) -> Result<(), Error> {
    let revision = i64::try_from(c.expected_job_revision).map_err(|_| Error::LinkConflict)?;
    let row=tx.query_opt("SELECT 1 FROM imagery.image_job j WHERE job_id=$1 AND owner_scope=$2 \
        AND state='recipe_frozen' AND revision=$3 AND absolute_deadline_ms=$4 \
        AND absolute_deadline_ms>$5 AND resolved_recipe_key=$6 AND recipe_revision=$7 \
        AND recipe_bytes=$8 AND config_revision=$9 AND config_bytes=$10 \
        AND EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=j.job_id AND i.state='attached') FOR UPDATE",
        &[&b.scope.job.0.as_slice(),&scope,&revision,&c.absolute_deadline_ms,&admitted_at_ms,
          &c.recipe_key.as_bytes().as_slice(),&i64::from(c.recipe.revision()),&c.recipe.bytes(),
          &i64::from(c.config.revision()),&c.config.bytes()]).await.map_err(failure)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    Ok(())
}

pub(super) async fn insert(
    tx: &Transaction<'_>,
    c: &Config,
    scope: &[u8],
    b: &Binding,
    r: &Receipt,
) -> Result<(), Error> {
    tx.execute("INSERT INTO imagery.image_publication_allocation(publication_attempt,asset_id,job_id, \
        stage_id,owner_scope,original_recipe_key,original_recipe_revision,original_recipe_bytes, \
        original_config_revision,original_config_bytes,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        &[&b.publication.0.as_slice(),&b.asset.0.as_slice(),&b.scope.job.0.as_slice(),&b.stage.0.as_slice(),
          &scope,&c.recipe_key.as_bytes().as_slice(),&i64::from(c.recipe.revision()),&c.recipe.bytes(),
          &i64::from(c.config.revision()),&c.config.bytes(),&r.admitted_at_ms]).await.map_err(failure)?;
    tx.execute("INSERT INTO imagery.image_finance_admission_binding(finance_attempt,publication_attempt, \
        job_id,stage_id,asset_id,owner_scope,account_id,operation_id,effect_id,receipt_namespace, \
        receipt_fingerprint,reservation_id,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'draft_admission',$10,$11,$12)",
        &[&r.attempt.as_bytes().as_slice(),&b.publication.0.as_slice(),&b.scope.job.0.as_slice(),
          &b.stage.0.as_slice(),&b.asset.0.as_slice(),&scope,&b.scope.account.as_bytes().as_slice(),
          &b.operation.as_slice(),&b.effect.as_slice(),&r.fingerprint.as_slice(),
          &r.reservation.as_bytes().as_slice(),&r.admitted_at_ms]).await.map_err(failure)?;
    Ok(())
}

pub(super) async fn verify_link(
    tx: &Transaction<'_>,
    c: &Config,
    scope: &[u8],
    b: &Binding,
    r: &Receipt,
) -> Result<(), Error> {
    let row=tx.query_opt("SELECT 1 FROM imagery.image_finance_admission_binding f \
        JOIN imagery.image_publication_allocation a USING(publication_attempt,job_id,stage_id,asset_id,owner_scope) \
        WHERE f.finance_attempt=$1 AND f.publication_attempt=$2 AND f.job_id=$3 AND f.stage_id=$4 \
        AND f.asset_id=$5 AND f.owner_scope=$6 AND f.account_id=$7 AND f.operation_id=$8 \
        AND f.effect_id=$9 AND f.receipt_namespace='draft_admission' AND f.receipt_fingerprint=$10 \
        AND f.reservation_id=$11 AND a.original_recipe_key=$12 AND a.original_recipe_revision=$13 \
        AND a.original_recipe_bytes=$14 AND a.original_config_revision=$15 AND a.original_config_bytes=$16",
        &[&r.attempt.as_bytes().as_slice(),&b.publication.0.as_slice(),&b.scope.job.0.as_slice(),
          &b.stage.0.as_slice(),&b.asset.0.as_slice(),&scope,&b.scope.account.as_bytes().as_slice(),
          &b.operation.as_slice(),&b.effect.as_slice(),&r.fingerprint.as_slice(),
          &r.reservation.as_bytes().as_slice(),&c.recipe_key.as_bytes().as_slice(),
          &i64::from(c.recipe.revision()),&c.recipe.bytes(),&i64::from(c.config.revision()),&c.config.bytes()])
        .await.map_err(failure)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    Ok(())
}
