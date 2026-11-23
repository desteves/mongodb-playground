variable "project_id" { type = string }
variable "domain" { type = string }
variable "notification_email" { type = string }
variable "billing_account" { type = string }
variable "budget_usd" { type = number }

resource "google_monitoring_notification_channel" "email" {
  count        = var.notification_email == "" ? 0 : 1
  project      = var.project_id
  display_name = "Playground alerts"
  type         = "email"
  labels       = { email_address = var.notification_email }
}

resource "google_monitoring_uptime_check_config" "health" {
  project      = var.project_id
  display_name = "Playground /readyz (reaches MongoDB)"
  timeout      = "10s"
  period       = "300s"
  http_check {
    path         = "/readyz"
    port         = 443
    use_ssl      = true
    validate_ssl = true
  }
  monitored_resource {
    type   = "uptime_url"
    labels = { project_id = var.project_id, host = var.domain }
  }
}

resource "google_monitoring_alert_policy" "down" {
  count        = var.notification_email == "" ? 0 : 1
  project      = var.project_id
  display_name = "Playground is down"
  combiner     = "OR"
  conditions {
    display_name = "Uptime check failing"
    condition_threshold {
      filter          = "metric.type=\"monitoring.googleapis.com/uptime_check/check_passed\" AND metric.label.check_id=\"${google_monitoring_uptime_check_config.health.uptime_check_id}\" AND resource.type=\"uptime_url\""
      comparison      = "COMPARISON_GT"
      threshold_value = 1
      duration        = "300s"
      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_NEXT_OLDER"
        cross_series_reducer = "REDUCE_COUNT_FALSE"
        group_by_fields      = ["resource.label.*"]
      }
    }
  }
  notification_channels = [google_monitoring_notification_channel.email[0].id]
}

# The API logs this warning (from the scheduled rollup job) when stored share links pass 80 % of SHARES_MAX_TOTAL.
resource "google_monitoring_alert_policy" "shares_nearly_full" {
  count        = var.notification_email == "" ? 0 : 1
  project      = var.project_id
  display_name = "Playground share storage is above 80 % of its cap"
  combiner     = "OR"
  conditions {
    display_name = "Share storage warning in the API log"
    condition_matched_log {
      filter = "resource.type=\"cloud_run_revision\" AND jsonPayload.fields.message:\"share storage is above 80\""
    }
  }
  alert_strategy {
    # One email a day at most while it stays above the line.
    notification_rate_limit {
      period = "86400s"
    }
  }
  documentation {
    content   = "Stored shares passed 80 % of SHARES_MAX_TOTAL. Raise the cap, or remove abusive shares with DELETE /internal/shares/<id>."
    mime_type = "text/markdown"
  }
  notification_channels = [google_monitoring_notification_channel.email[0].id]
}

resource "google_billing_budget" "monthly" {
  count           = var.billing_account == "" ? 0 : 1
  billing_account = var.billing_account
  display_name    = "MongoDB Playground"
  amount {
    specified_amount {
      currency_code = "USD"
      units         = tostring(var.budget_usd)
    }
  }
  threshold_rules { threshold_percent = 0.5 }
  threshold_rules { threshold_percent = 0.9 }
  threshold_rules { threshold_percent = 1.0 }
  dynamic "all_updates_rule" {
    for_each = var.notification_email == "" ? [] : [1]
    content {
      monitoring_notification_channels = [google_monitoring_notification_channel.email[0].id]
    }
  }
}
