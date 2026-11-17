import { execFileSync } from "node:child_process";

// The bucket predates this stack. Merge only our named delivery rule so an
// existing transport restriction or unrelated policy is never replaced.
function aws(args) {
  return JSON.parse(
    execFileSync("aws", [...args, "--region", "us-east-1", "--output", "json"], { encoding: "utf8" }) || "{}",
  );
}
const account = "572186105468";
if (aws(["sts", "get-caller-identity"]).Account !== account) throw new Error(`Expected account ${account}`);
const stack = aws(["cloudformation", "describe-stacks", "--stack-name", "xfalcon-desktop-updates-staging"]).Stacks[0];
if (!["CREATE_COMPLETE", "UPDATE_COMPLETE"].includes(stack.StackStatus)) throw new Error("Delivery stack is not ready");
const distribution = stack.Outputs.find((output) => output.OutputKey === "DistributionId")?.OutputValue;
if (!distribution || !/^[A-Z0-9]+$/.test(distribution)) throw new Error("Missing distribution ID");
const bucket = "xfalcon-desktop-artifacts-staging";
const policy = JSON.parse(
  aws(["s3api", "get-bucket-policy", "--bucket", bucket, "--expected-bucket-owner", account]).Policy,
);
policy.Statement = [
  ...policy.Statement.filter((statement) => statement.Sid !== "CloudFrontDeliveryOnly"),
  {
    Sid: "CloudFrontDeliveryOnly",
    Effect: "Allow",
    Principal: { Service: "cloudfront.amazonaws.com" },
    Action: "s3:GetObject",
    Resource: [
      `arn:aws:s3:::${bucket}/desktop/releases/*`,
      `arn:aws:s3:::${bucket}/desktop/channels/staging/latest.json`,
    ],
    Condition: { StringEquals: { "AWS:SourceArn": `arn:aws:cloudfront::${account}:distribution/${distribution}` } },
  },
];
aws([
  "s3api",
  "put-bucket-policy",
  "--bucket",
  bucket,
  "--expected-bucket-owner",
  account,
  "--policy",
  JSON.stringify(policy),
]);
console.log(`Configured private origin for ${distribution}`);
