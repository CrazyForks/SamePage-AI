-- AlterTable
ALTER TABLE "DocumentCollaborationLinkInvite" ADD COLUMN "passwordCode" TEXT;
ALTER TABLE "DocumentCollaborationLinkInvite" ADD COLUMN "passwordEnabled" BOOLEAN NOT NULL DEFAULT false;

UPDATE "DocumentCollaborationLinkInvite"
SET "passwordEnabled" = true
WHERE "passwordHash" IS NOT NULL;
