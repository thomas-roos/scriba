class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.15.2"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.2/scriba-x86_64-apple-darwin"
    sha256 "9b60fd52e8f9cc9f394c8be0571638eef3667ade841f235f915179057d33493b"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.2/scriba-aarch64-apple-darwin"
    sha256 "fa22dad084fe785068788a8f380b8ddb3afb45646523e2dfa106272b70f945ec"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
