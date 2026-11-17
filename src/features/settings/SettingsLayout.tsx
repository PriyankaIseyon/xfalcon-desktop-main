import React from "react";
import { cn } from "../../lib/cn";
import { HighlightedText, SearchHighlightScope } from "../../lib/highlightedText";
import { useIsSearchRevealTarget } from "../../lib/searchReveal";

export interface SettingsPageProps extends React.HTMLAttributes<HTMLDivElement> {
  contentClassName?: string;
  ref?: React.Ref<HTMLDivElement>;
}

export function SettingsPage({ className, contentClassName, children, ref, ...props }: SettingsPageProps) {
  return (
    <div ref={ref} className={cn("settings-page", className)} data-settings-page="" {...props}>
      <div className={cn("settings-page__content", contentClassName)} data-settings-page-content="">
        {children}
      </div>
    </div>
  );
}

export interface SettingsSectionProps extends Omit<React.HTMLAttributes<HTMLElement>, "title"> {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
  headingId?: string;
}

export function SettingsSection({
  title,
  description,
  actions,
  headingId,
  className,
  children,
  ...props
}: SettingsSectionProps) {
  const generatedHeadingId = React.useId();
  const resolvedHeadingId = headingId ?? generatedHeadingId;
  const { id, ...sectionProps } = props;
  const revealed = useIsSearchRevealTarget(id);

  return (
    <section
      id={id}
      aria-labelledby={resolvedHeadingId}
      className={cn("settings-section", className)}
      data-settings-section=""
      data-search-reveal={revealed ? "" : undefined}
      {...sectionProps}
    >
      <SearchHighlightScope active={revealed}>
        <header className="settings-section__header">
          <div className="settings-section__copy">
            <h3 className="settings-section__title" id={resolvedHeadingId}>
              {typeof title === "string" ? <HighlightedText text={title} /> : title}
            </h3>
            {description ? (
              <p className="settings-section__description">
                {typeof description === "string" ? <HighlightedText text={description} /> : description}
              </p>
            ) : null}
          </div>
          {actions ? <SettingsActions>{actions}</SettingsActions> : null}
        </header>
        {children}
      </SearchHighlightScope>
    </section>
  );
}

/**
 * True inside another SettingsGroup. A nested group draws no box of its own:
 * the outer group carries the border, and nested blocks are separated by a
 * hairline (see `.settings-group--nested` in index.css).
 */
const SettingsGroupNestingContext = React.createContext(false);

export function SettingsGroup({ className, children, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  const nested = React.useContext(SettingsGroupNestingContext);
  const revealed = useIsSearchRevealTarget(props.id);
  return (
    <SettingsGroupNestingContext.Provider value={true}>
      <div
        role="group"
        className={cn("settings-group", nested ? "settings-group--nested" : undefined, className)}
        data-settings-group=""
        data-settings-group-nested={nested ? "" : undefined}
        data-search-reveal={revealed ? "" : undefined}
        {...props}
      >
        {revealed ? <SearchHighlightScope active>{children}</SearchHighlightScope> : children}
      </div>
    </SettingsGroupNestingContext.Provider>
  );
}

export interface SettingsGroupHeaderProps extends Omit<React.HTMLAttributes<HTMLDivElement>, "title"> {
  title: React.ReactNode;
  description?: React.ReactNode;
  headingId?: string;
}

export function SettingsGroupHeader({
  title,
  description,
  headingId,
  className,
  children,
  ...props
}: SettingsGroupHeaderProps) {
  return (
    <div className={cn("settings-group-header", className)} data-settings-group-header="" {...props}>
      <div className="settings-group-header__copy" data-settings-group-header-copy="">
        <h4 className="settings-group-header__title" id={headingId}>
          {typeof title === "string" ? <HighlightedText text={title} /> : title}
        </h4>
        {description ? (
          <p className="settings-group-header__description">
            {typeof description === "string" ? <HighlightedText text={description} /> : description}
          </p>
        ) : null}
      </div>
      {children ? <SettingsActions>{children}</SettingsActions> : null}
    </div>
  );
}

export interface SettingsRowProps extends Omit<React.HTMLAttributes<HTMLDivElement>, "title"> {
  label: React.ReactNode;
  hint?: React.ReactNode;
  labelId?: string;
}

export function SettingsRow({ label, hint, labelId, className, children, ...props }: SettingsRowProps) {
  return (
    <div className={cn("settings-row", className)} data-settings-row="" {...props}>
      <div className="settings-row__copy" data-settings-row-copy="">
        <div className="settings-row__label" id={labelId}>
          {typeof label === "string" ? <HighlightedText text={label} /> : label}
        </div>
        {hint ? (
          <div className="settings-row__hint">{typeof hint === "string" ? <HighlightedText text={hint} /> : hint}</div>
        ) : null}
      </div>
      <div className="settings-row__control" data-settings-row-control="">
        {children}
      </div>
    </div>
  );
}

export function SettingsActions({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("settings-actions", className)} data-settings-actions="" {...props} />;
}
