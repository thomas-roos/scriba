class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.14.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.14.0/scriba-x86_64-apple-darwin"
    sha256 "33dae052a4ead70351385faa1561513aee4937639f46dcee524ef1120287302d"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.14.0/scriba-aarch64-apple-darwin"
    sha256 "3700b0fcaeb8b48e3edea3336225abd4eb45aa7ae4a8655c7bda22fd450d2379"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
