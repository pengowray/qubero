package net.pengo.hexdraw.original.renderer;

import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.HexPanel;

public class SeperatorRenderer implements Renderer {

    private	String seperator = "  ";
    
    private boolean	render;
    private HexPanel hexpanel;
    
    public int renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            int columnWidth ) {
    	FontMetrics fm = g.getFontMetrics();
        g.drawString(seperator, 0, fm.getAscent() );
        return fm.bytesWidth(seperator.getBytes(), 0, seperator.length()); 
    }
    
    
    public SeperatorRenderer(HexPanel hexpanel, boolean render) {
        super();
        this.hexpanel = hexpanel;
        setEnabled(render);
    }
    public boolean isEnabled() {
        return render;
    }
    public void setEnabled(boolean render) {
        this.render = render;
        hexpanel.repaint();
   	}
    
    public HexPanel getPanel() {
        return hexpanel;
    }

    public String toString() {
        return "_";
    }

}


/*
 renderer = getViewMode( "Hex" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 if (selected)  {
 g.setColor( new Color(170,170,255) ); // FIXME: cache
 g.fillRect( lineStart + hexStart[j], (int)(lineHeight*linenum), hexStart[j+1]-hexStart[j], lineHeight);  //FIXME: precision loss!
 g.fillRect( lineStart + hexStart[j], (int)(lineHeight*linenum), hexStart[j+1]-hexStart[j], lineHeight);  //FIXME: precision loss!
 }
 g.setColor(Color.black);
 g.drawString( byte2hex(b), lineStart + hexStart[j], (int)(charAscent + lineHeight*linenum)); //FIXME: precision loss!
 }
 //g.drawString( byte2hex(b), (float)lineStart + hexStart[j], (float)(charAscent + lineHeight*linenum)); //FIXME: precision loss?
 */

/*
 renderer = getViewMode( "Grey scale" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 int col = ~((int)b) & 0xff; // the "gamma" of grey squares
 if (selected)  {
 g.setColor(new Color(col/3*2,col/3*2,col/2+128));
 }
 else  {
 g.setColor(new Color(col,col,col));
 }
 g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth-1, lineHeight-1); //FIXME: precision loss!
 
 
 //g.setColor(Color.red);
 //g.drawString( byte2ascii(b), lineStart + hexStart[hexPerLine] + (charWidth*j), charAscent + lineHeight*linenum );
 }
 */

/*
 renderer = getViewMode( "Grey scale II" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 int col = ~((int)b) & 0xf0; // the gamma of left/outer square
 int col2 = (~((int)b) & 0x0f) << 4; // the gamma of right/inner square
 col = col | (col >> 4); // turn a0 into aa
 col2 = col2 | (col2 >> 4);
 
 if (selected)  {
 g.setColor(new Color(col/3*2,col/3*2,col/2+128));
 g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth-1, lineHeight-1); //FIXME: precision loss!
 g.setColor(new Color(col2/3*2,col2/3*2,col2/2+128));
 g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j) + (charWidth/2), (int)(lineHeight*linenum)+(lineHeight/2), (charWidth/2)-1, (lineHeight/2)-1); //FIXME: precision loss!
 }
 else  {
 g.setColor(new Color(col,col,col));
 g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth-1, lineHeight-1); //FIXME: precision loss!
 g.setColor(new Color(col2,col2,col2));
 g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j) + (charWidth/2), (int)(lineHeight*linenum)+(lineHeight/2), (charWidth/2)-1, (lineHeight/2)-1); //FIXME: precision loss!
 }
 }
 */

/*
 renderer = getViewMode( "Ascii" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 if (selected)  {
 g.setColor( new Color(170,170,255) ); // FIXME: cache
 g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth, lineHeight); //FIXME: precision loss!
 }
 g.setColor( Color.black );
 g.drawString( byte2ascii(b), lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum + charAscent) ); //FIXME: precision loss!
 }
 //ascii.append(byte2ascii(b));
 */

/*
 renderer = getViewMode( "Wave" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 
 // side wave form 
 int waveLengthStart = lineStart + hexStart[hexPerLine] + (charWidth*3) +100;
 int waveTopHeight =  (int)(lineHeight*linenum)-charHeight;
 
 float yPt = (16) / lineHeight; // y point distance
 int waveOffset = (int) (lineHeight + (yPt*j));
 
 //int coll = b; //(int)((((int)b) & 0xff)) / maxHor; //b;
 int coll = ((int)b & 0xff);
 
 if (selected) {
 g.setColor(new Color(170,170,255));
 } else {
 g.setColor(Color.darkGray);
 }
 g.drawLine(waveLengthStart, waveTopHeight+waveOffset, waveLengthStart+coll, waveTopHeight+waveOffset);
 //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
 }
 */

/*
 renderer = getViewMode( "Wave RGB" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 // side rgb form 
 int rgbLengthStart = lineStart + hexStart[hexPerLine] + (charWidth*3) +200;
 int rgbTopHeight =  (int)(lineHeight*linenum)-charHeight;
 
 float yPt = (16) / lineHeight; // y point distance
 int rgbOffset = (int) (lineHeight + (yPt*j));
 
 //int coll = b; //(int)((((int)b) & 0xff)) / maxHor; //b;
 int coll = ((int)b & 0xff);
 
 if (selected) {
 g.setColor(new Color(170,170,255));
 } else {
 g.setColor(Color.darkGray);
 }
 g.drawLine(waveLengthStart, waveTopHeight+waveOffset, waveLengthStart+coll, waveTopHeight+waveOffset);
 //g.drawLine(0, waveTopHeight+waveOffset, 0+coll, waveTopHeight+waveOffset);
 }
 */

/*                    
 renderer = getViewMode( "Address" );
 //System.out.println("renderer="+renderer +":" +renderer.isEnabled());
 if ( renderer != null && renderer.isEnabled() ) {
 //// draw address:
 //g.setColor(Color.black);
 //String addr = BigInteger.valueOf(i).toString(16)+":  ";
 //g.setColor(Color.darkGray);
 //g.drawString( addr,lineStart - fm.stringWidth(addr),(int)(lineHeight*linenum + charAscent)); //FIXME: precision loss!
 }
 */                
