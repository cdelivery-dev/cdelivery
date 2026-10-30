mod service;

pub(crate) mod typ;

use crate::{concept, usecase};

macro_rules! channels {
    ($consumer:ident) => {
        $consumer! {
            email {
                get { GetEmail, SettingsEmail, "settings_get_email", "Get the email channel settings." }
                set { SetEmail, set_email, UpdateEmailSettingsRequest, EmailSettings, resolve_email, "settings_set_email", "Patch and verify the email channel settings." }
                clear { ClearEmail, "settings_clear_email", "Clear the email channel settings." }
                path "/settings/channels/email"
            }
            feishu {
                get { GetFeishu, SettingsFeishu, "settings_get_feishu", "Get the Feishu channel settings." }
                set { SetFeishu, set_feishu, UpdateFeishuSettingsRequest, FeishuSettings, resolve_feishu, "settings_set_feishu", "Patch and verify the Feishu channel settings." }
                clear { ClearFeishu, "settings_clear_feishu", "Clear the Feishu channel settings." }
                path "/settings/channels/feishu"
            }
            lark {
                get { GetLark, SettingsLark, "settings_get_lark", "Get the Lark channel settings." }
                set { SetLark, set_lark, UpdateFeishuSettingsRequest, FeishuSettings, resolve_feishu, "settings_set_lark", "Patch and verify the Lark channel settings." }
                clear { ClearLark, "settings_clear_lark", "Clear the Lark channel settings." }
                path "/settings/channels/lark"
            }
            slack {
                get { GetSlack, SettingsSlack, "settings_get_slack", "Get the Slack channel settings." }
                set { SetSlack, set_slack, UpdateSlackSettingsRequest, SlackSettings, resolve_slack, "settings_set_slack", "Patch and verify Slack settings. Requires Socket Mode, message.im/app_mention events, bot scopes chat:write/im:history/app_mentions:read (bot_token, xoxb-), connections:write (app_token, xapp-), and the App Home messages tab enabled, all in one app." }
                clear { ClearSlack, "settings_clear_slack", "Clear the Slack channel settings." }
                path "/settings/channels/slack"
            }
            telegram {
                get { GetTelegram, SettingsTelegram, "settings_get_telegram", "Get the Telegram channel settings." }
                set { SetTelegram, set_telegram, UpdateBotSettingsRequest, BotSettings, resolve_bot, "settings_set_telegram", "Patch and verify Telegram settings. Use a dedicated bot token; startup removes its webhook. Long polling accepts direct messages, /cdelivery@BotUsername commands, and replies to the bot; ignores topics." }
                clear { ClearTelegram, "settings_clear_telegram", "Clear the Telegram channel settings." }
                path "/settings/channels/telegram"
            }
            discord {
                get { GetDiscord, SettingsDiscord, "settings_get_discord", "Get the Discord channel settings." }
                set { SetDiscord, set_discord, UpdateBotSettingsRequest, BotSettings, resolve_bot, "settings_set_discord", "Patch and verify Discord settings. Requires View Channels, Send Messages, and Send Messages in Threads; no privileged Message Content intent." }
                clear { ClearDiscord, "settings_clear_discord", "Clear the Discord channel settings." }
                path "/settings/channels/discord"
            }
        }
    };
}

pub(crate) use channels;

macro_rules! define_channels {
    ($($channel:ident {
        get { $get:ident, $wrapper:ident, $get_id:tt, $get_description:tt }
        set { $set:ident, $store:ident, $request:ident, $response:ident, $resolve:ident, $set_id:tt, $set_description:tt }
        clear { $clear:ident, $clear_id:tt, $clear_description:tt }
        path $path:tt
    })+) => {$(
        super::define!(
            $get, super::input::Empty => typ::$wrapper,
            $get_id, $get_description, Shared,
            super::Effect::Read, Standard, super::Media::Json;
            |app, _access, _input| {
                Ok(typ::$wrapper {
                    $channel: usecase::setting::get(&app.docker, &app.key)
                        .await?
                        .$channel()
                        .map(Into::into),
                })
            }
        );

        super::define!(
            $set, typ::$request => typ::$response,
            $set_id, $set_description, Admin,
            super::Effect::Write { destructive: true, idempotent: true },
            Extended, super::Media::Json;
            |app, _access, input| {
                let value = usecase::setting::$store(
                    &app.docker,
                    &app.key,
                    &app.agent_reload,
                    |current| service::$resolve(input, current),
                )
                .await?;
                Ok(typ::$response::from(&value))
            }
        );

        super::define!(
            $clear, super::input::Empty => super::Done,
            $clear_id, $clear_description, Admin,
            super::Effect::Write { destructive: true, idempotent: true },
            Standard, super::Media::Json;
            |app, _access, _input| {
                usecase::setting::clear(
                    &app.docker,
                    &app.key,
                    &app.agent_reload,
                    concept::setting::Settings::$store,
                )
                .await?;
                Ok(super::Done::new())
            }
        );
    )+};
}

channels!(define_channels);

super::define!(
    GetAgent, super::input::Empty => typ::SettingsAgent,
    "settings_get_agent", "Get AI Agent settings.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        let settings = usecase::setting::get(&app.docker, &app.key).await?;
        let duty_enabled = !settings.duty_paused();
        Ok(typ::SettingsAgent {
            agent: settings
                .agent()
                .map(|agent| typ::AgentSettings::assemble(agent, duty_enabled))
                .transpose()?,
        })
    }
);

super::define!(
    SetAgent, typ::UpdateAgentSettingsRequest => typ::AgentSettings,
    "settings_set_agent", "Patch and verify AI Agent settings. duty_enabled controls AI analysis, patrols, actions, and notifications; ordinary alerts are independent. language sets generated Duty notification language.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Extended, super::Media::Json;
    |app, _access, input| {
        let duty_enabled = input.duty_enabled;
        let (agent, duty_enabled) = usecase::setting::set_agent(
            &app.docker,
            &app.key,
            &app.agent_reload,
            duty_enabled,
            |current| service::resolve_agent(input, current),
        )
        .await?;
        typ::AgentSettings::assemble(&agent, duty_enabled)
    }
);

super::define!(
    ClearAgent, super::input::Empty => super::Done,
    "settings_clear_agent", "Clear AI Agent and Duty settings.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, _input| {
        usecase::setting::clear(
            &app.docker,
            &app.key,
            &app.agent_reload,
            concept::setting::Settings::set_agent,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    DiscoverModels, typ::DiscoverAgentModelsRequest => typ::AgentModels,
    "settings_discover_models", "List an AI provider's models.", Admin,
    super::Effect::Read, Extended, super::Media::Json;
    |app, _access, input| {
        let settings = usecase::setting::get(&app.docker, &app.key).await?;
        let (provider, api_key, endpoint) = service::resolve_discovery(input, settings.agent())?;
        Ok(typ::AgentModels {
            models: usecase::setting::discover_models(
                provider,
                api_key.as_deref(),
                endpoint.as_deref(),
            )
            .await?,
        })
    }
);
