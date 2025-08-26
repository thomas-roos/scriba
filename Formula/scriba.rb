class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.7.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.7.0/scriba-x86_64-apple-darwin"
    sha256 "bf9d4ffa29e0f82633e2591e785f6303c05befd4e4b00584f2a3389b58e34bf6"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.7.0/scriba-aarch64-apple-darwin"
    sha256 "529dbadd3bed2f281faafd9dfed812d36694a2dbe3e5271320dd7bf1ea5301b0"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
