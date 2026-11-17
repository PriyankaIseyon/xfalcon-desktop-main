import React from "react";
import { Bell, FileText, Search, Trash2 } from "lucide-react";
import { Accordion } from "./ui/Accordion";
import { Alert } from "./ui/Alert";
import { Avatar } from "./ui/Avatar";
import { Badge } from "./ui/Badge";
import { Breadcrumbs } from "./ui/Breadcrumbs";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { Checkbox } from "./ui/Checkbox";
import { Chip } from "./ui/Chip";
import { DateField, DateTimeField, TimeField } from "./ui/DateTimeField";
import { Divider } from "./ui/Divider";
import { EmptyState } from "./ui/EmptyState";
import { Field, Input, Textarea } from "./ui/Field";
import { FileField } from "./ui/FileField";
import { Eyebrow, HeroTitle, PageTitle, SectionLabel, SectionTitle } from "./ui/Heading";
import { IconButton } from "./ui/IconButton";
import { Kbd, Shortcut } from "./ui/Kbd";
import { Label } from "./ui/Label";
import { Link } from "./ui/Link";
import { List, ListItem } from "./ui/List";
import { Logo } from "./ui/Logo";
import { Menu } from "./ui/Menu";
import { NavBar } from "./ui/NavBar";
import { Pagination } from "./ui/Pagination";
import { Panel } from "./ui/Panel";
import { Progress } from "./ui/Progress";
import { Radio, RadioGroup } from "./ui/Radio";
import { SearchField } from "./ui/SearchField";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Select } from "./ui/Select";
import { Sidebar } from "./ui/Sidebar";
import { Skeleton } from "./ui/Skeleton";
import { Slider } from "./ui/Slider";
import { Spinner } from "./ui/Spinner";
import { StatusDot } from "./ui/StatusDot";
import { Switch } from "./ui/Switch";
import { Table, TableBody, TableHead, TableRow, Td, Th } from "./ui/Table";
import { Tabs } from "./ui/Tabs";
import { Tooltip } from "./ui/Tooltip";
import { XFalconMark } from "./ui/XFalconMark";

/**
 * Every primitive, on one page, so the nine display modes in task 1.16 can be
 * verified against the real thing rather than against a description of it.
 *
 * # Why this is not part of the app
 *
 * `primitives.html` is deliberately absent from `build.rollupOptions.input`, so
 * the dev server serves this page and a production build never emits it.
 * `check-style-contract.mjs` fails if it ever reaches `dist`. A gallery is a
 * developer tool; shipping one would put a second, unreachable surface in the
 * bundle a user downloads.
 *
 * # What a specimen owes
 *
 * `data-primitive` is the P-id from the closure ledger and `data-specimen`
 * names the case. The sweep in `tests/visual/primitives.spec.ts` walks these
 * attributes, so a primitive that is not here is not verified — and the ledger
 * says as much rather than claiming coverage it does not have.
 */
export function Specimen({ id, name, children }: { id: string; name: string; children: React.ReactNode }) {
  return (
    <section
      className="flex flex-col gap-2 rounded-lg border border-hairline p-3"
      data-primitive={id}
      data-specimen={name}
    >
      <span className="type-micro text-faint">
        {id} · {name}
      </span>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </section>
  );
}

export function Gallery() {
  const [text, setText] = React.useState("Quarterly");
  const [choice, setChoice] = React.useState("first");
  const [tab, setTab] = React.useState("one");
  const [mode, setMode] = React.useState("alpha");
  const [on, setOn] = React.useState(true);
  const [page, setPage] = React.useState(2);
  const [option, setOption] = React.useState<string | undefined>("a");

  return (
    <main className="flex flex-col gap-4 p-6 text-foreground">
      <Specimen id="P01" name="Button">
        <Button variant="primary">Primary</Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="destructive">Delete</Button>
        <Button disabled>Disabled</Button>
        <Button loading>Saving</Button>
        <Button shape="pill" variant="primary">
          Pill
        </Button>
      </Specimen>

      <Specimen id="P02" name="IconButton">
        <IconButton aria-label="Notifications">
          <Bell aria-hidden />
        </IconButton>
        <IconButton aria-label="Delete row" variant="danger">
          <Trash2 aria-hidden />
        </IconButton>
        <IconButton aria-label="Send" variant="primary">
          <Bell aria-hidden />
        </IconButton>
        <IconButton aria-label="Unavailable" disabled>
          <Bell aria-hidden />
        </IconButton>
      </Specimen>

      <Specimen id="P03" name="Link">
        <Link href="https://xfalcon.ai/docs">External docs</Link>
        <Link href="/projects">In-app destination</Link>
        <Link href="/projects" variant="muted">
          Muted
        </Link>
      </Specimen>

      <Specimen id="P04" name="Field">
        <Field hint="Use your work address" label="Email">
          <Input onChange={(event) => setText(event.target.value)} value={text} />
        </Field>
        <Field error="That address is not valid" label="Invalid" required>
          <Input defaultValue="nope" />
        </Field>
        <Field label="Notes">
          <Textarea defaultValue="Prose" rows={2} />
        </Field>
      </Specimen>

      <Specimen id="P05" name="FileField">
        <FileField label="Choose file" />
      </Specimen>

      <Specimen id="P06" name="SearchField">
        <SearchField
          aria-label="Search reports"
          onChange={(event) => setText(event.target.value)}
          onClear={() => setText("")}
          value={text}
        />
      </Specimen>

      <Specimen id="P07" name="DateTimeField">
        <DateField aria-label="Date" defaultValue="2026-08-26" />
        <TimeField aria-label="Time" defaultValue="09:30" />
        <DateTimeField aria-label="When" defaultValue="2026-08-26T09:30" />
      </Specimen>

      <Specimen id="P08" name="Select">
        <Field label="Worksheet">
          <Select
            onChange={setOption}
            options={[
              { label: "Sheet one", value: "a" },
              { label: "Sheet two", value: "b" },
            ]}
            value={option}
          />
        </Field>
      </Specimen>

      <Specimen id="P09" name="Checkbox">
        <Checkbox aria-label="Unchecked" />
        <Checkbox aria-label="Checked" checked readOnly />
        <Checkbox aria-label="Mixed" indeterminate />
        <Checkbox aria-label="Disabled" disabled />
      </Specimen>

      <Specimen id="P10" name="Radio">
        <RadioGroup label="Mode" name="gallery-mode" onValueChange={setMode} value={mode}>
          <Radio value="alpha">Alpha</Radio>
          <Radio value="beta">Beta</Radio>
        </RadioGroup>
      </Specimen>

      <Specimen id="P11" name="Switch">
        <Switch checked={on} label="Reasoning" onChange={setOn} />
        <Switch checked={false} label="Off" onChange={() => {}} />
        <Switch busy checked={false} label="Busy" onChange={() => {}} />
      </Specimen>

      <Specimen id="P12" name="Slider">
        <Slider defaultValue={4} label="Detail" max={10} min={0} step={1} />
      </Specimen>

      <Specimen id="P13" name="SegmentedControl">
        <SegmentedControl
          aria-label="View"
          onChange={setChoice}
          options={[
            { label: "First", value: "first" },
            { label: "Second", value: "second" },
          ]}
          value={choice}
        />
      </Specimen>

      <Specimen id="P14" name="Tabs">
        <Tabs
          aria-label="Sections"
          onChange={setTab}
          tabs={[
            { id: "one", label: "One" },
            { id: "two", label: "Two" },
          ]}
          value={tab}
        >
          <p className="type-caption text-muted">Panel body for {tab}</p>
        </Tabs>
      </Specimen>

      <Specimen id="P15" name="Accordion">
        <Accordion items={[{ children: "The details", id: "details", open: true, title: "Details" }]} />
      </Specimen>

      <Specimen id="P16" name="Menu">
        <Menu trigger={<Button variant="secondary">Actions</Button>}>
          <Menu.Item>Rename</Menu.Item>
          <Menu.Separator />
          <Menu.Item variant="danger">Delete</Menu.Item>
        </Menu>
      </Specimen>

      {/* P17 ContextMenu is the same portalled panel P16 opens, so opening the
          menu above is what puts it on screen for the sweep to measure. */}
      <Specimen id="P17" name="ContextMenu">
        <span className="type-caption text-muted">Shares the panel opened by P16.</span>
      </Specimen>

      <Specimen id="P18" name="Tooltip">
        <Tooltip label="Saved to this device">
          <Button variant="ghost">Hover me</Button>
        </Tooltip>
      </Specimen>

      {/* P19 Modal and P20 AlertDialog own the viewport when open, so the sweep
          opens them one at a time rather than parking them in the flow. */}
      <Specimen id="P19" name="Modal">
        <span className="type-caption text-muted">Opened by the sweep.</span>
      </Specimen>
      <Specimen id="P20" name="AlertDialog">
        <span className="type-caption text-muted">Opened by the sweep.</span>
      </Specimen>

      <Specimen id="P21" name="Alert">
        <Alert tone="info" title="Heads up">
          An informational consequence.
        </Alert>
        <Alert tone="success">Saved.</Alert>
        <Alert tone="warn">Check the format.</Alert>
        <Alert action={<Button size="sm">Retry</Button>} tone="danger">
          Import failed.
        </Alert>
      </Specimen>

      <Specimen id="P22" name="Toaster">
        <span className="type-caption text-muted">Raised by the sweep.</span>
      </Specimen>

      <Specimen id="P23" name="Progress">
        <Progress label="Uploading" value={38} />
        <Progress label="Complete" tone="success" value={100} />
        <Progress indeterminate label="Preparing" value={0} />
      </Specimen>

      <Specimen id="P24" name="Skeleton">
        <Skeleton className="h-8 w-40" />
      </Specimen>

      <Specimen id="P25" name="Spinner">
        <Spinner label="Reading schema" />
        <Spinner decorative />
      </Specimen>

      <Specimen id="P26" name="StatusDot">
        <StatusDot label="Online" tone="online" />
        <StatusDot label="Sync failed" tone="error" />
        <StatusDot decorative tone="accent" />
      </Specimen>

      <Specimen id="P27" name="EmptyState">
        <EmptyState
          actions={<Button variant="primary">Upload dataset</Button>}
          description="Add a CSV or Excel file to start."
          icon={<FileText aria-hidden />}
          title="No datasets yet"
        />
      </Specimen>

      <Specimen id="P28" name="Card">
        <Card className="p-4">Resting card</Card>
        <Card className="p-4" interactive onClick={() => {}}>
          Interactive card
        </Card>
      </Specimen>

      <Specimen id="P29" name="Panel">
        <Panel padded>Panel body</Panel>
      </Specimen>

      <Specimen id="P30" name="Table">
        <Table>
          <TableHead>
            <TableRow>
              <Th>Region</Th>
              <Th>Revenue</Th>
            </TableRow>
          </TableHead>
          <TableBody>
            <TableRow>
              <Td>EMEA</Td>
              <Td>1,240</Td>
            </TableRow>
          </TableBody>
        </Table>
      </Specimen>

      <Specimen id="P31" name="List">
        <List aria-label="Servers">
          <ListItem>Alpha</ListItem>
          <ListItem>Beta</ListItem>
        </List>
      </Specimen>

      <Specimen id="P32" name="NavBar">
        <NavBar variant="plain">Top chrome</NavBar>
      </Specimen>

      <Specimen id="P33" name="Sidebar">
        <Sidebar aria-label="Workspace navigation">
          <Button variant="ghost">Projects</Button>
        </Sidebar>
      </Specimen>

      <Specimen id="P34" name="Breadcrumbs">
        <Breadcrumbs
          items={[{ href: "/projects", label: "Projects" }, { label: "Reindex", onClick: () => {} }, { label: "Here" }]}
        />
      </Specimen>

      <Specimen id="P35" name="Pagination">
        <Pagination onPageChange={setPage} page={page} pageCount={5} />
      </Specimen>

      <Specimen id="P36" name="Heading">
        <HeroTitle>Hero title</HeroTitle>
        <PageTitle>Page title</PageTitle>
        <SectionTitle>Section title</SectionTitle>
        <SectionLabel>Section label</SectionLabel>
        <Eyebrow>Eyebrow</Eyebrow>
      </Specimen>

      <Specimen id="P37" name="Label">
        <Label htmlFor="gallery-label-target" required>
          Named field
        </Label>
        <Input id="gallery-label-target" />
      </Specimen>

      <Specimen id="P38" name="Divider">
        <Divider />
        <Divider orientation="vertical" />
      </Specimen>

      <Specimen id="P39" name="Badge and Chip">
        <Badge tone="neutral">Neutral</Badge>
        <Badge tone="accent">Accent</Badge>
        <Badge tone="success">Success</Badge>
        <Badge tone="warn">Warn</Badge>
        <Badge tone="danger">Danger</Badge>
        <Chip>Interactive</Chip>
        <Chip disabled>Disabled</Chip>
        <Chip interactive={false}>Static</Chip>
      </Specimen>

      <Specimen id="P40" name="Avatar">
        <Avatar size="sm">JT</Avatar>
        <Avatar size="md">JT</Avatar>
        <Avatar size="lg">JT</Avatar>
      </Specimen>

      <Specimen id="P41" name="Kbd">
        <Shortcut keys={["Mod", "K"]} />
        <Shortcut keys={["Mod", "Shift", "F"]} />
        <Kbd>Tab</Kbd>
      </Specimen>

      <Specimen id="P42" name="Logo">
        <Logo subtitle="Desktop" />
        <span className="app-mark">
          <XFalconMark label="xFalcon" />
        </span>
      </Specimen>

      <span className="sr-only">
        <Search aria-hidden />
      </span>
    </main>
  );
}
