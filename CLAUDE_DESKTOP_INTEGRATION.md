# Claude Desktop Integration for Scriba MCP Server

## Overview

Scriba now includes a Model Context Protocol (MCP) server that integrates seamlessly with Claude Desktop, allowing you to access your audio transcripts directly in your conversations with Claude.

## Features

The Scriba MCP server provides the following tools:

- **`list_transcripts`**: List all recordings with transcripts (newest first)
- **`get_transcript`**: Fetch the full transcript content by recording ID or directory name  
- **`search_transcripts`**: Full-text search across all transcripts using SQLite FTS
- **`get_recording_info`**: Get detailed metadata about specific recordings

## Installation & Setup

### 1. Build Scriba with MCP Support

Ensure you have the latest version of Scriba built:

```bash
cd scriba
cargo build --release
```

### 2. Configure Claude Desktop

Edit your Claude Desktop configuration file to add the Scriba MCP server:

#### macOS Configuration

Edit `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "scriba": {
      "command": "/path/to/scriba",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

#### Windows Configuration

Edit `%APPDATA%/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "scriba": {
      "command": "C:\\path\\to\\scriba.exe",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

#### Linux Configuration

Edit `~/.config/claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "scriba": {
      "command": "/path/to/scriba",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

**Important**: Replace the path above with your actual Scriba installation path. Use `which scriba` to find the correct path on your system.

### 3. Restart Claude Desktop

After updating the configuration, completely restart Claude Desktop to load the MCP server.

## Usage Examples

Once configured, you can use Scriba's tools in your Claude conversations:

### List Recent Transcripts

```
Can you list my recent transcripts using the list_transcripts tool?
```

### Search Through Transcripts  

```
Please search my transcripts for discussions about "machine learning" using the search_transcripts tool.
```

### Get a Specific Transcript

```
Can you get the transcript for recording ID 5 using the get_transcript tool?
```

### Get Recording Details

```
Show me the detailed information for the recording in directory "meeting-2024-01-15" using get_recording_info.
```

## Tool Reference

### `list_transcripts`

Lists recordings with transcripts, newest first.

**Parameters:**
- `limit` (optional): Maximum number of items to return
- `offset` (optional): Number of items to skip  
- `include_without_transcripts` (optional): Include recordings without transcripts (default: false)

**Example:**
```json
{
  "limit": 10,
  "offset": 0,
  "include_without_transcripts": false
}
```

### `get_transcript`

Fetches the full transcript content for a specific recording.

**Parameters:**
- `recording_id` (optional): Recording ID to fetch transcript for
- `directory_name` (optional): Directory name to fetch transcript for

**Note:** Provide either `recording_id` OR `directory_name`.

**Example:**
```json
{
  "recording_id": 42
}
```

### `search_transcripts`

Full-text search across all transcripts using SQLite FTS.

**Parameters:**
- `query` (required): Search query string
- `limit` (optional): Maximum number of results to return

**Example:**
```json
{
  "query": "quarterly meeting budget",
  "limit": 5
}
```

### `get_recording_info`

Gets detailed metadata about a specific recording.

**Parameters:**
- `recording_id` (optional): Recording ID
- `directory_name` (optional): Directory name

**Note:** Provide either `recording_id` OR `directory_name`.

**Example:**
```json
{
  "directory_name": "standup-2024-01-15"
}
```

## Troubleshooting

### MCP Server Not Starting

1. **Check the path**: Ensure the path to the Scriba executable is correct in your configuration
2. **Test manually**: Try running `scriba mcp` directly to see if there are any errors
3. **Check permissions**: Make sure the Scriba executable has proper permissions
4. **View logs**: Check Claude Desktop's logs for any MCP-related errors

### Database Issues

If you encounter database errors:

1. **Run health check**: Execute `scriba health --verbose` to check database status
2. **Database location**: The MCP server uses the same database as the main Scriba application (`~/scriba_recordings/scriba.db`)
3. **Permissions**: Ensure the database directory is writable

### No Results Returned

If tools return empty results:

1. **Check data**: Verify you have recordings with transcripts using `scriba` (TUI mode)
2. **Database path**: Ensure MCP server can access the same database as your recordings
3. **Search syntax**: For `search_transcripts`, try simpler search terms

## Advanced Configuration

### Custom Database Path

If you need to specify a custom database path, you can set environment variables:

```json
{
  "mcpServers": {
    "scriba": {
      "command": "/path/to/scriba/target/release/scriba",
      "args": ["mcp"],
      "env": {
        "SCRIBA_DB_PATH": "/custom/path/to/scriba.db"
      }
    }
  }
}
```

### Performance Tuning

The MCP server automatically enables performance optimizations when running in MCP mode:

- Skips heavy database integrity checks
- Uses optimized query patterns for list operations
- Implements efficient search indexing

## Security Notes

- The MCP server only provides read-access to your transcripts
- No recording or modification capabilities are exposed through MCP
- All communication happens locally between Claude Desktop and the MCP server
- Your transcript data never leaves your local machine

## Support

If you encounter issues:

1. Check this documentation first
2. Run `scriba health --verbose` to verify your installation
3. Test the MCP server directly: `scriba mcp`
4. Check Claude Desktop's documentation for MCP troubleshooting
5. File issues on the [Scriba GitHub repository](https://github.com/giovannialberto/scriba)

## Version Compatibility

- **Scriba**: 0.11.1+
- **Claude Desktop**: Latest version with MCP support
- **MCP Protocol**: 2024-11-05