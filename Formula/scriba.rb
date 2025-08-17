class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.5.4"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.4/scriba-x86_64-apple-darwin"
    sha256 "60f76005f0289daf4f08dd70739612083a0646412ecef13980478717c27650ec"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.4/scriba-aarch64-apple-darwin"
    sha256 "39b2cc5087a0ccbc94c1e67192b32901d78cf45fef1704a70464b80d1436c4f9"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
