class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.5.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.0/scriba-x86_64-apple-darwin"
    sha256 "074f2af9f1fbf380dcbbd732eeb1644125481a92ba692969d81fd5b6bf6d10cb"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.0/scriba-aarch64-apple-darwin"
    sha256 "074f2af9f1fbf380dcbbd732eeb1644125481a92ba692969d81fd5b6bf6d10cb"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end