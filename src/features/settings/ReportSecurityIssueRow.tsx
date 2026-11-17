import React from "react";
import { ShieldAlert } from "lucide-react";
import { usePlatform } from "../../lib/platform";
import { Link } from "../../ui/Link";
import { SettingsRow } from "./SettingsLayout";
import { SECURITY_REPORT_ADDRESS, securityReportMailto } from "./securityReport";

/**
 * A `mailto:` link, not a command: the existing opener hands it to the mail
 * app. It is styled as the `SettingsButton` secondary control, so it reads as
 * an action beside the other Settings buttons. `variant="nav"` keeps `Link`
 * from adding its hover underline, which a button does not have.
 */
export function ReportSecurityIssueRow({ applicationVersion }: { applicationVersion: string }) {
  const platform = usePlatform();
  const href = securityReportMailto({ applicationVersion, platform, now: new Date() });

  return (
    <SettingsRow
      id="settings-security-report"
      label="Report a security issue"
      hint={`Opens your mail app with a prefilled, urgent message to ${SECURITY_REPORT_ADDRESS}. Do not include passwords or customer data.`}
    >
      <Link
        href={href}
        variant="nav"
        className="settings-control settings-control--radius settings-button settings-button--wide"
        data-settings-control="button"
        data-settings-control-size="32"
        data-settings-variant="secondary"
      >
        <span
          aria-hidden="true"
          className="settings-control__icon"
          data-settings-control-icon=""
          data-settings-icon-size="16"
        >
          <ShieldAlert />
        </span>
        Report a security issue
      </Link>
    </SettingsRow>
  );
}
