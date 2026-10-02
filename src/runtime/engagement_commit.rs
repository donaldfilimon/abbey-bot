//! Engagement uses Work's retained canonical commit owner.
impl super::AppState {
    pub async fn commit_engagement<R: Send + 'static>(
        &self,
        change: impl FnOnce(
            &mut crate::engagement::EngagementStore,
        ) -> Result<R, crate::work::WorkError>
        + Send
        + 'static,
    ) -> Result<R, crate::work::WorkError> {
        self.commit_work(move |store| {
            let value = change(&mut store.engagement)?;
            store.engagement.validate()?;
            Ok(value)
        })
        .await
    }
}
