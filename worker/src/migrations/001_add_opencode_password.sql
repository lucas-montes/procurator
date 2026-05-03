-- Add opencode_password column to ip_leases table
-- This stores the per-VM password for OpenCode server authentication
ALTER TABLE ip_leases ADD COLUMN IF NOT EXISTS opencode_password TEXT;
