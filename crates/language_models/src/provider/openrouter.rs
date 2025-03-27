use anyhow::{anyhow, Context as _, Result};
use credentials_provider::CredentialsProvider;
use editor::{Editor, EditorElement};
use gpui::{Context, Entity, FontStyle, Subscription, Task, TextStyle, WhiteSpace};
use http_client::HttpClient;
use language_model::{
    AuthenticateError, LanguageModel, LanguageModelId, LanguageModelProvider,
    LanguageModelProviderId, LanguageModelProviderName, LanguageModelProviderState, RateLimiter,
};
use settings::{Settings, SettingsStore};
use theme::ThemeSettings;
use ui::{prelude::*, List, Render};
use util::ResultExt;

use crate::{ui::InstructionListItem, AllLanguageModelSettings};

const PROVIDER_ID: &str = "openrouter";
const PROVIDER_NAME: &str = "OpenRouter";

#[derive(Default, Clone, Debug, PartialEq)]
pub struct OpenRouterSettings {
    pub api_url: String,
    pub available_models: Vec<AvailableModel>,
    pub needs_setting_migration: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: usize,
    pub max_output_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
}

pub struct OpenRouterLanguageModelProvider {
    http_client: Arc<dyn HttpClient>,
    state: gpui::Entity<State>,
}

pub struct State {
    api_key: Option<String>,
    api_key_from_env: bool,
    _subscription: Subscription,
}

const OPENROUTER_API_KEY_VAR: &str = "OPENROUTER_API_KEY";

impl State {
    fn is_authenticated(&self) -> bool {
        self.api_key.is_some()
    }

    fn reset_api_key(&self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let credentials_provider = <dyn CredentialsProvider>::global(cx);
        let api_url = AllLanguageModelSettings::get_global(cx)
            .openrouter
            .api_url
            .clone();
        cx.spawn(async move |this, cx| {
            credentials_provider
                .delete_credentials(&api_url, &cx)
                .await
                .log_err();
            this.update(cx, |this, cx| {
                this.api_key = None;
                this.api_key_from_env = false;
                cx.notify();
            })
        })
    }

    fn set_api_key(&mut self, api_key: String, cx: &mut Context<Self>) -> Task<Result<()>> {
        let credentials_provider = <dyn CredentialsProvider>::global(cx);
        let api_url = AllLanguageModelSettings::get_global(cx)
            .openrouter
            .api_url
            .clone();
        cx.spawn(async move |this, cx| {
            credentials_provider
                .write_credentials(&api_url, "Bearer", api_key.as_bytes(), &cx)
                .await
                .log_err();
            this.update(cx, |this, cx| {
                this.api_key = Some(api_key);
                cx.notify();
            })
        })
    }

    fn authenticate(&self, cx: &mut Context<Self>) -> Task<Result<(), AuthenticateError>> {
        if self.is_authenticated() {
            return Task::ready(Ok(()));
        }

        let credentials_provider = <dyn CredentialsProvider>::global(cx);
        let api_url = AllLanguageModelSettings::get_global(cx)
            .openrouter
            .api_url
            .clone();
        cx.spawn(async move |this, cx| {
            let (api_key, from_env) = std::env::var(OPENROUTER_API_KEY_VAR)
                .map(|key| (key, true))
                .or_else(async |_| {
                    credentials_provider
                        .read_credentials(&api_url, &cx)
                        .await?
                        .ok_or(AuthenticateError::CredentialsNotFound)
                        .and_then(|(_, api_key)| {
                            String::from_utf8(api_key)
                                .map(|key| (key, false))
                                .context("invalid {PROVIDER_NAME} API key")
                        })
                })?;
            this.update(cx, |this, cx| {
                this.api_key = Some(api_key);
                this.api_key_from_env = from_env;
                cx.notify();
            })?;

            Ok(())
        })
    }
}

impl OpenRouterLanguageModelProvider {
    pub fn new(http_client: Arc<dyn HttpClient>, cx: &mut App) -> Self {
        let state = cx.new(|cx| State {
            api_key: None,
            api_key_from_env: false,
            _subscription: cx.observe_global::<SettingsStore>(|_this: &mut State, cx| {
                cx.notify();
            }),
        });

        Self { http_client, state }
    }
}

impl LanguageModelProviderState for OpenRouterLanguageModelProvider {
    type ObservableEntity = State;

    fn observable_entity(&self) -> Option<gpui::Entity<Self::ObservableEntity>> {
        Some(self.state.clone())
    }
}

impl LanguageModelProvider for OpenRouterLanguageModelProvider {
    fn id(&self) -> LanguageModelProviderId {
        LanguageModelProviderId(PROVIDER_ID.into())
    }

    fn name(&self) -> LanguageModelProviderName {
        LanguageModelProviderName(PROVIDER_NAME.into())
    }

    fn icon(&self) -> IconName {
        IconName::AiOpenRouter
    }

    fn default_model(&self, cx: &ui::App) -> Option<std::sync::Arc<dyn LanguageModel>> {
        // it's good to use openrouter_rs to get user's default model on openrouter settings page
        // but currently, openrouter doesn't provide a way to get the user's default model
        // so we'll just use the hard-coded default model
        //
        // let model = openrouter_rs::Model::default();
        Some(Arc::new(OpenRouterLanguageModel {
            id: LanguageModelId::from(model.id().to_string()),
            // model,
            state: self.state.clone(),
            http_client: self.http_client.clone(),
            request_limiter: RateLimiter::new(4),
        }))
    }

    fn provided_models(&self, cx: &ui::App) -> Vec<std::sync::Arc<dyn LanguageModel>> {
        // get models from openrouter_rs
        // but, openrouter offers too many models
        // it's better to select a fixed range of models in ConfigurationView
        // and models selected in ConfigurationView is the final provided models
        // also, only use models which support tools
        // TODO: add models selector in ConfigurationView
        todo!()
    }

    fn is_authenticated(&self, cx: &ui::App) -> bool {
        self.state.read(cx).is_authenticated()
    }

    fn authenticate(&self, cx: &mut ui::App) -> gpui::Task<gpui::Result<(), AuthenticateError>> {
        self.state.update(cx, |state, cx| state.authenticate(cx))
    }

    fn configuration_view(&self, window: &mut ui::Window, cx: &mut ui::App) -> gpui::AnyView {
        cx.new(|cx| ConfigurationView::new(self.state.clone(), window, cx))
            .into()
    }

    fn reset_credentials(&self, cx: &mut ui::App) -> gpui::Task<gpui::Result<()>> {
        self.state.update(cx, |state, cx| state.reset_api_key(cx))
    }
}

pub struct OpenRouterLanguageModel {
    id: LanguageModelId,
    // model: openrouter_rs::Model,
    state: gpui::Entity<State>,
    http_client: Arc<dyn HttpClient>,
    request_limiter: RateLimiter,
}

impl LanguageModel for OpenRouterLanguageModel {
    fn id(&self) -> language_model::LanguageModelId {
        self.id.clone()
    }

    fn name(&self) -> language_model::LanguageModelName {
        // LanguageModelName::from(self.model.display_name().to_string())
    }

    fn provider_id(&self) -> LanguageModelProviderId {
        LanguageModelProviderId(PROVIDER_ID.into())
    }

    fn provider_name(&self) -> LanguageModelProviderName {
        LanguageModelProviderName(PROVIDER_NAME.into())
    }

    fn telemetry_id(&self) -> String {
        // format!("openrouter/{}", self.model.id())
    }

    fn max_token_count(&self) -> usize {
        // self.model.max_token_count()
    }

    fn max_output_tokens(&self) -> Option<u32> {
        // self.model.max_output_tokens()
    }

    fn count_tokens(
        &self,
        request: language_model::LanguageModelRequest,
        cx: &ui::App,
    ) -> futures::future::BoxFuture<'static, gpui::Result<usize>> {
        // call openrouter_rs::get_generation() to count tokens
        todo!()
    }

    fn stream_completion(
        &self,
        request: language_model::LanguageModelRequest,
        cx: &gpui::AsyncApp,
    ) -> futures::future::BoxFuture<
        'static,
        gpui::Result<
            futures::stream::BoxStream<
                'static,
                gpui::Result<language_model::LanguageModelCompletionEvent>,
            >,
        >,
    > {
        todo!()
    }

    fn use_any_tool(
        &self,
        request: language_model::LanguageModelRequest,
        name: String,
        description: String,
        schema: serde_json::Value,
        cx: &gpui::AsyncApp,
    ) -> futures::future::BoxFuture<
        'static,
        gpui::Result<futures::stream::BoxStream<'static, gpui::Result<String>>>,
    > {
        todo!()
    }
}

struct ConfigurationView {
    api_key_editor: Entity<Editor>,
    state: gpui::Entity<State>,
    load_credentials_task: Option<Task<()>>,
}

impl ConfigurationView {
    fn new(state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let api_key_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text(
                "sk-or-v1-0000000000000000000000000000000000000000000000000000000000000000",
                cx,
            );
            editor
        });

        cx.observe(&state, |_, _, cx| {
            cx.notify();
        })
        .detach();

        let load_credentials_task = Some(cx.spawn_in(window, {
            let state = state.clone();
            async move |this, cx| {
                if let Some(task) = state
                    .update(cx, |state, cx| state.authenticate(cx))
                    .log_err()
                {
                    // We don't log an error, because "not signed in" is also an error.
                    let _ = task.await;
                }

                this.update(cx, |this, cx| {
                    this.load_credentials_task = None;
                    cx.notify();
                })
                .log_err();
            }
        }));

        Self {
            api_key_editor,
            state,
            load_credentials_task,
        }
    }

    fn save_api_key(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let api_key = self.api_key_editor.read(cx).text(cx);
        if api_key.is_empty() {
            return;
        }

        let state = self.state.clone();
        cx.spawn_in(window, async move |_, cx| {
            state
                .update(cx, |state, cx| state.set_api_key(api_key, cx))?
                .await
        })
        .detach_and_log_err(cx);

        cx.notify();
    }

    fn reset_api_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.api_key_editor
            .update(cx, |editor, cx| editor.set_text("", window, cx));

        let state = self.state.clone();
        cx.spawn_in(window, async move |_, cx| {
            state.update(cx, |state, cx| state.reset_api_key(cx))?.await
        })
        .detach_and_log_err(cx);

        cx.notify();
    }

    fn render_api_key_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = ThemeSettings::get_global(cx);
        let text_style = TextStyle {
            color: cx.theme().colors().text,
            font_family: settings.ui_font.family.clone(),
            font_features: settings.ui_font.features.clone(),
            font_fallbacks: settings.ui_font.fallbacks.clone(),
            font_size: rems(0.875).into(),
            font_weight: settings.ui_font.weight,
            font_style: FontStyle::Normal,
            line_height: relative(1.3),
            white_space: WhiteSpace::Normal,
            ..Default::default()
        };
        EditorElement::new(
            &self.api_key_editor,
            EditorStyle {
                background: cx.theme().colors().editor_background,
                local_player: cx.theme().players().local(),
                text: text_style,
                ..Default::default()
            },
        )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> impl ui::IntoElement {
        v_flex()
          .size_full()
          .on_action(cx.listener(Self::save_api_key))
          .child(Label::new("To use Zed's assistant with OpenRouter, you need to add an API key. Follow these steps:"))
          .child(
              List::new()
                  .child(InstructionListItem::new(
                      "Create one by visiting",
                      Some("OpenRouter's console"),
                      Some("https://openrouter.ai/settings/keys"),
                  ))
                  .child(InstructionListItem::text_only(
                      "Ensure your OpenRouter account has credits",
                  ))
                  .child(InstructionListItem::text_only(
                      "Paste your API key below and hit enter to start using the assistant",
                  )),
          )
          .child(
              h_flex()
                  .w_full()
                  .my_2()
                  .px_2()
                  .py_1()
                  .bg(cx.theme().colors().editor_background)
                  .border_1()
                  .border_color(cx.theme().colors().border_variant)
                  .rounded_sm()
                  .child(self.render_api_key_editor(cx)),
          )
          .child(
              Label::new(
                  format!("You can also assign the {OPENROUTER_API_KEY_VAR} environment variable and restart Zed."),
              )
              .size(LabelSize::Small).color(Color::Muted),
          )
          .into_any()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl ui::IntoElement {
        h_flex()
          .size_full()
          .justify_between()
          .child(
            h_flex()
              .gap_1()
              .child(Icon::new(IconName::Check).color(Color::Success))
              .child(Label::new(match env_var_set {
                true => format!("API key set in {OPENROUTER_API_KEY_VAR} environment variable."),
                false => "API key configured.".to_string(),
              })),
          )
          .child(
            Button::new("reset-key", "Reset key")
              .icon(Some(IconName::Trash))
              .icon_size(IconSize::Small)
              .icon_position(IconPosition::Start)
              .disabled(env_var_set)
              .when(env_var_set, |this| {
                this.tooltip(Tooltip::text(format!("To reset your API key, unset the {OPENROUTER_API_KEY_VAR} environment variable.")))
              })
              .on_click(cx.listener(|this, _, window, cx| this.reset_api_key(window, cx))),
          )
          // TODO: add model selector for OpenRouter
          .into_any()
    }

    fn should_render_editor(&self, cx: &mut Context<Self>) -> bool {
        !self.state.read(cx).is_authenticated()
    }
}

impl Render for ConfigurationView {
    fn render(
        &mut self,
        window: &mut ui::Window,
        cx: &mut ui::Context<'_, Self>,
    ) -> impl ui::IntoElement {
        let env_var_set = self.state.read(cx).api_key_from_env;

        match (self.load_credentials_task, self.should_render_editor(cx)) {
            (None, true) => self.render_editor(cx),
            (None, false) => self.render_settings(cx),
            (Some(_), _) => div().child(Label::new("Loading credentials...")).into_any(),
        }
    }
}
