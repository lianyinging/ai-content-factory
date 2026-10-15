mod assets;
mod browser_cdp;
mod catbus;
mod competitors;
mod dashboard;
mod drafts;
mod fs_dialog;
mod image_jobs;
mod knowledge;
mod library;
mod llm;
mod media_cache;
mod node_jobs;
mod products;

use catbus::CatbusState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(CatbusState::default())
        .manage(browser_cdp::BrowserProcess(std::sync::Mutex::new(
            Default::default(),
        )))
        .setup(|app| {
            let lib = library::init_library(app.handle())
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(lib);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<browser_cdp::BrowserProcess>()
                {
                    browser_cdp::kill_all_browsers(&state);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            catbus::catbus_doctor,
            catbus::catbus_run,
            catbus::catbus_qr_start,
            catbus::catbus_qr_status,
            catbus::catbus_qr_image,
            library::library_save_items,
            library::library_list,
            library::library_delete,
            library::library_group_list,
            library::library_group_create,
            library::library_group_rename,
            library::library_group_delete,
            library::library_group_reorder,
            library::library_move_to_group,
            library::library_set_analysis,
            llm::list_llm_providers,
            llm::upsert_llm_provider,
            llm::delete_llm_provider,
            llm::set_default_llm_provider,
            llm::get_llm_runtime_by_modality,
            llm::test_llm_provider,
            llm::fetch_llm_balance,
            drafts::draft_list,
            drafts::draft_get,
            drafts::draft_upsert,
            drafts::draft_delete,
            drafts::library_to_drafts,
            products::product_list,
            products::product_upsert,
            products::product_delete,
            drafts::publish_log_list,
            drafts::publish_log_add,
            drafts::publish_log_clear,
            assets::asset_dir_info,
            assets::asset_list,
            assets::asset_get,
            assets::asset_ingest,
            assets::asset_delete,
            assets::asset_rename,
            assets::asset_reveal,
            assets::asset_move,
            assets::asset_reorder,
            assets::media_folder_list,
            assets::media_folder_create,
            assets::media_folder_rename,
            assets::media_folder_delete,
            assets::media_folder_get,
            assets::media_folder_reorder,
            assets::media_folder_move,
            image_jobs::image_job_start,
            image_jobs::image_job_list,
            image_jobs::image_job_get,
            image_jobs::image_job_delete,
            fs_dialog::save_text_with_dialog,
            fs_dialog::read_text_file,
            competitors::competitor_list,
            competitors::competitor_add,
            competitors::competitor_delete,
            competitors::competitor_refresh,
            competitors::competitor_snapshots,
            competitors::competitor_snapshots_backfill,
            competitors::competitor_set_chat,
            competitors::competitor_set_analysis,
            competitors::competitor_comments,
            competitors::competitor_refresh_logs,
            competitors::competitor_get_settings,
            competitors::competitor_put_settings,
            competitors::competitor_scan_opportunities,
            competitors::competitor_list_opportunities,
            competitors::competitor_delete_opportunity,
            competitors::competitor_set_keywords,
            dashboard::dashboard_insight_config_get,
            dashboard::dashboard_insight_config_upsert,
            dashboard::industry_insight_run_add,
            dashboard::industry_insight_run_latest,
            competitors::watched_user_list,
            competitors::watched_user_add,
            competitors::watched_user_delete,
            competitors::watched_user_refresh,
            competitors::watched_user_events,
            competitors::watched_user_mark_read,
            browser_cdp::detect_browsers,
            browser_cdp::get_browser_config,
            browser_cdp::save_browser_config,
            browser_cdp::list_browser_profiles,
            browser_cdp::upsert_browser_profile,
            browser_cdp::delete_browser_profile,
            browser_cdp::update_browser_profile_session,
            browser_cdp::browser_cdp_status,
            browser_cdp::browser_cdp_status_all,
            browser_cdp::launch_browser_cdp,
            browser_cdp::stop_browser_cdp,
            browser_cdp::login_browser_platform,
            browser_cdp::open_browser_url,
            browser_cdp::set_browser_ai_banner,
            browser_cdp::list_browser_tabs,
            browser_cdp::probe_xhs_message_badge,
            browser_cdp::read_xhs_unread_messages,
            browser_cdp::probe_xhs_im_state,
            browser_cdp::click_xhs_conv_item,
            browser_cdp::send_xhs_chat_reply,
            node_jobs::ai_run,
            node_jobs::cdp_run,
            knowledge::kb_run,
            knowledge::auto_ops_get_binding,
            knowledge::auto_ops_upsert_binding,
            knowledge::auto_ops_log_append,
            knowledge::auto_ops_log_list,
            knowledge::acq_reply_history_list,
            knowledge::acq_reply_history_clear,
            knowledge::cs_session_list,
            knowledge::cs_message_list,
            knowledge::cs_message_append,
            knowledge::cs_messages_sync,
            knowledge::cs_session_touch,
            browser_cdp::probe_xhs_conv_list,
            browser_cdp::probe_xhs_filter_panel,
            browser_cdp::probe_xhs_comment_match,
            browser_cdp::probe_xhs_custom_reply,
            browser_cdp::xhs_acq_run,
            browser_cdp::xhs_acq_force_idle,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
