/// Finance-history field setter table.
///
/// Mirrors Go's `internal/service/finance_history.go`.
/// Uses a declarative table of (fieldId, setter-fn) to avoid repetitive if/else chains.
use crate::client::mcs::{
    VerifyFinanceHistoryReq, VerifyPlayerFinanceInfo, VerifyPlayerHistoryInfo,
};
use crate::model::merchant_rule::Question;
use std::collections::HashMap;

/// A single field setter.
pub struct FhFieldSetter {
    pub field_id: &'static str,
    pub apply: fn(&mut VerifyFinanceHistoryReq, String, String),
}

// ── Finance field setters ─────────────────────────────────────────────────────

pub static FINANCE_SETTERS: &[FhFieldSetter] = &[
    FhFieldSetter {
        field_id: "BANK_ACCOUNT",
        apply: |r, v, _| r.verify_player_finance_info.bc_number = v,
    },
    FhFieldSetter {
        field_id: "CARD_HOLDER_NAME",
        apply: |r, v, _| r.verify_player_finance_info.bc_holder_name = v,
    },
    FhFieldSetter {
        field_id: "VIRTUAL_WALLET_ADDRESS",
        apply: |r, v, _| r.verify_player_finance_info.vw_address = v,
    },
    FhFieldSetter {
        field_id: "VIRTUAL_WALLET_NAME",
        apply: |r, v, _| r.verify_player_finance_info.vw_holder_name = v,
    },
    FhFieldSetter {
        field_id: "E_WALLET_ACCOUNT",
        apply: |r, v, _| r.verify_player_finance_info.ew_account = v,
    },
    FhFieldSetter {
        field_id: "E_WALLET_NAME",
        apply: |r, v, _| r.verify_player_finance_info.ew_holder_name = v,
    },
];

// ── History field setters ─────────────────────────────────────────────────────

pub static HISTORY_SETTERS: &[FhFieldSetter] = &[
    FhFieldSetter {
        field_id: "LAST_DEPOSIT_AMOUNT",
        apply: |r, v, acc| {
            r.verify_player_history_info.last_deposit_amount = v;
            r.verify_player_history_info.last_deposit_amount_range = acc;
        },
    },
    FhFieldSetter {
        field_id: "LAST_DEPOSIT_TIME",
        apply: |r, v, acc| {
            r.verify_player_history_info.last_deposit_time = v;
            r.verify_player_history_info.last_deposit_time_range_in_day = acc.trim().parse().unwrap_or(0);
        },
    },
    FhFieldSetter {
        field_id: "LAST_DEPOSIT_METHOD",
        apply: |r, v, _| r.verify_player_history_info.last_deposit_method = v,
    },
    FhFieldSetter {
        field_id: "LAST_WITHDRAWAL_AMOUNT",
        apply: |r, v, acc| {
            r.verify_player_history_info.last_withdraw_amount = v;
            r.verify_player_history_info.last_withdraw_amount_range = acc;
        },
    },
    FhFieldSetter {
        field_id: "LAST_WITHDRAWAL_TIME",
        apply: |r, v, acc| {
            r.verify_player_history_info.last_withdraw_time = v;
            r.verify_player_history_info.last_withdraw_time_range_in_day = acc.trim().parse().unwrap_or(0);
        },
    },
    FhFieldSetter {
        field_id: "LAST_WITHDRAWAL_METHOD",
        apply: |r, v, _| r.verify_player_history_info.last_withdraw_method = v,
    },
];

// ── Apply logic ───────────────────────────────────────────────────────────────

/// Apply a group of field setters with consistent bind/value/accuracy logic:
///
/// - bind=false                         → value="",     accuracy=""
/// - bind=true  && fieldIDMap value=="" → value="NULL", accuracy="NULL"
/// - bind=true  && fieldIDMap value!="" → value=actual, accuracy from question config
pub fn apply_field_setters(
    setters: &[FhFieldSetter],
    req: &mut VerifyFinanceHistoryReq,
    field_id_map: &HashMap<String, String>,
    bind_map: &HashMap<String, bool>,
    question_cfg: &HashMap<String, Question>,
) {
    for s in setters {
        let bound = *bind_map.get(s.field_id).unwrap_or(&false);

        if !bound {
            // 玩家未绑定该字段，置空
            (s.apply)(req, String::new(), String::new());
            continue;
        }

        let field_value = field_id_map
            .get(s.field_id)
            .map(|s| s.as_str())
            .unwrap_or("");

        if field_value.is_empty() {
            // 已绑定但未填写，MCS 约定用 "NULL" 表示空值
            (s.apply)(req, "NULL".to_string(), "NULL".to_string());
            continue;
        }

        // 已绑定且有实际值，accuracy 取问题配置
        let accuracy = question_cfg
            .get(s.field_id)
            .map(|q| q.accuracy.clone())
            .unwrap_or_default();
        (s.apply)(req, field_value.to_string(), accuracy);
    }
}
