import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;
import java.awt.geom.*;
import java.io.*;
import java.awt.dnd.*;
import java.awt.datatransfer.*;
import java.math.*;

class HexPanel extends JPanel implements OpenFileListener, Scrollable, DropTargetListener { 
    protected OpenFile openFile = null;
    protected Data root;
    protected long rootLength; // cached value to check if it's changed size.
    
    protected boolean dimensionsCalculated = false;
    protected long height; // height of entire "sheet" of hex
    protected int viewportHeight = 500; // preferred viewport height
    protected int width; // width of entire "sheet" of hex
    protected int lineStart; // where does the first character start
    protected int hexPerLine = 16; // number of hex units shown per line
    protected int[] hexStart; // where each hex starts on a line.
    protected int unitWidth; // length of two characters, basically.
    protected int lineHeight; // height of one line of hex
    protected long totalLines;
    protected boolean charSizeKnown = false;
    //protected int charWidth = 8, charHeight = 14; // laptop sizes
    protected int charWidth = 7, charHeight = 16, charAscent = 12; // probable sizes(?)
    
    protected Data selection = null;
    protected int cursor = -1;

    private boolean draggingMode = false; // mouse is currently dragging a selection
    private boolean published = false; // has the current selection been published? 
    
    Font font = new Font("Monospaced", Font.PLAIN, 11); // antialias?
    
    private int greyMode = 0;

    public HexPanel(OpenFile openFile) {
	super();
        setBackground(Color.white);
        
	setOpenFile(openFile);
	
        //XXX: openFile's resources should be considered in rendering ?

	// MOUSE ROUTINES

        addMouseListener(
            new MouseInputAdapter() {
                public void mousePressed(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
                    int hclick = hexFromClick( e.getX(), e.getY() );
                    if (hclick != -1) {
                        setSelection(hclick, 1, false);
			draggingMode = true;
                    } else {
			draggingMode = false;
                    }		    

                }

                public void mouseReleased(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
		    if (draggingMode && selection != null) {
			setSelection(getSelection(), true);
			draggingMode = false;
		    }

                }

                public void mouseClicked(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
                    int hclick = hexFromClick( e.getX(), e.getY() );
                    if (hclick != -1) {
                        setSelection(hclick, 1, true);
                    }
		    draggingMode = false;
                }
            }
        );

        addMouseMotionListener(new MouseInputAdapter() {
            public void mouseDragged(MouseEvent e) {
                int hclick = hexFromClick( e.getX(), e.getY() );
                if (draggingMode && hclick != -1) {
		    long start = selection.getStart(); 
		    setSelection(start, (hclick-start)+1, false );
                }
        
            }
        });
        
    }
    
    public void setOpenFile(OpenFile openFile) {
	if (this.openFile != null) {
	    this.openFile.removeOpenFileListener(this);
	}

	this.openFile = openFile;
        this.root = openFile.getData();
        clearSelection();
	cursor = -1;
        rootLength = root.getLength();
        
        openFile.addOpenFileListener(this);
        
        reCalcDim();
        
        long MAX_PREC = 16777216;
        if (height > Integer.MAX_VALUE) {
            System.out.println("Warning: Very long file being displayed. Display height of " + height + " > Integer.MAX_VALUE (" + Integer.MAX_VALUE + ")" );
            System.out.println("      Size is " + (float)((double)height/Integer.MAX_VALUE)*100 + "% of Integer.MAX_VALUE" );
            System.out.println("      Size is " + (float)((double)height/MAX_PREC)*100 + "% of Max precise display height" );
        } else if (height > MAX_PREC ) {
            System.out.println("Note: Longish file being displayed. Max precise display height of " + MAX_PREC + " < actual display height of " + height + " < Integer.MAX_VALUE (" + Integer.MAX_VALUE + ")" );
            System.out.println("      Size is " + (float)((double)height/Integer.MAX_VALUE)*100 + "% of Integer.MAX_VALUE" );
            System.out.println("      Size is " + (float)((double)height/MAX_PREC)*100 + "% of Max precise display height" );
        }
        
    }
    
    public int hexFromClick(int x, int y) {

        int hx = -1;
        int hy = -1;

        for (int h=0; h<hexStart.length; h++) {
            if (x >= lineStart+hexStart[h] && x <= lineStart+hexStart[h]+unitWidth) {
                hx = h;
                break;
            }
        }
        if (hx == -1) {
            // not found
            return -1;
        }

        hy = y / lineHeight;

        return (hy * hexPerLine) + hx;      
    }

    
    public Data getSelection() {
	return selection;
    }

    public void clearSelection() {
        if (selection == null)
            return;
        
        Data oldSel = selection;
        selection = null;
        
        openFile.clearSelection(this);
        
        long minC = oldSel.getStart();
        long maxC = minC + oldSel.getLength();

        //repaint only necessary areas
        //xxx: this just gets converted back to hex units again, so why bother?
        //xxx: doesn't trim X-ways
        repaint(0, (int)((minC/hexPerLine)*lineHeight), width, (int)(((maxC/hexPerLine)+1)*(lineHeight))); //XXX: loss of precision!
    }

    public void setSelection(TransparentData sel) {
	setSelection(sel, true);
    }

    public synchronized void setSelection(Data sel, boolean publish) {
	if (sel.equals(selection) && (publish==false || published==true)) return;

        Data oldSel = this.selection;
	this.selection = sel;

	if (publish==true) {
	    openFile.setSelection(this, selection);
	    published = true;
	} else {
	    published = false;
	}

        
        long minC, maxC; // min and max characters changed
        if (oldSel != null) { 
            long nselStart = sel.getStart();
            long oselStart = oldSel.getStart();
            long nselEnd = nselStart + sel.getLength();
            long oselEnd = oselStart + oldSel.getLength();
            minC = Math.min(nselStart, oselStart);
            maxC = Math.max(nselEnd, oselEnd);
        } else {
            minC = sel.getStart();
            maxC = minC + sel.getLength();
        }

        //repaint only necessary areas
        //xxx: this just gets converted back to hex units again, so why bother?
        //xxx: doesn't trim X-ways
        repaint(0, (int)((minC/hexPerLine)*lineHeight), width, (int)(((maxC/hexPerLine)+1)*(lineHeight))); //XXX: loss of precision!
    }

    public void setSelection(long offset, long len) {
	setSelection(offset, len, true);
    }
    public void setSelection(long offset, long len, boolean publish) {
        if (offset >= root.getLength()) {
            // off the board!
            clearSelection();
            return;
        }
        if (offset + len > root.getLength()) {
            len = root.getLength() - offset;
        }
        setSelection(root.getSelection(offset, len), publish);
    }

    public Dimension getPreferredSize() {
        //FIXME: this is a hack!
	return new Dimension(width, (int)height); //XXX: precision loss!
        

    }

    public Dimension getMinimumSize() {
	return getPreferredSize();
    }


    public Dimension getMaximumSize() {
        return new Dimension(width, (int)height); //XXX: precision loss!
    }
    
    //XXX: will need def listener?
 
    protected void reCalcDim() {
        dimensionsCalculated = false;
        calcDim();
    }
    
    /** calculate dimensions */
    protected void calcDim() {
        if (dimensionsCalculated == true)
            return;

        //calcDim(getComponentGraphics().getFontMetrics(font)); // oops stupid no work
        
	calcDim(charWidth, charHeight); //XXX: may not have been worked out from FontMetrics
	if (charSizeKnown == false) {
	    // don't trust size until calculated with FontMetrics
	    dimensionsCalculated = false; 
	}
    }

    protected void calcDim(FontMetrics fm) {
        if (dimensionsCalculated == true)
            return;

	if (charSizeKnown == false) {
            int oldCharWidth = charWidth;
            int oldCharHeight = charHeight;
            int oldCharAscent = charAscent;
	    charWidth = fm.charWidth('W');
	    charHeight = fm.getHeight();
            charAscent = fm.getAscent();
	    charSizeKnown = true;
            if (oldCharWidth != charWidth || oldCharHeight != charHeight) {
                System.out.println("Note: screen character size different to expected: " + charWidth  + "x" + charHeight + ",ascent:" + charAscent +  " instead of expected: " + oldCharWidth  + "x" + oldCharHeight + ",ascent:" + oldCharAscent);
            }
            if (oldCharAscent != charAscent ) {
                System.out.println("Note: character ascent different to expected: " + charAscent + " instead of expected: " + oldCharAscent);
            }
	}

	calcDim(charWidth, charHeight);
    }

    protected synchronized void calcDim(int charWidth, int charHeight) {
        if (dimensionsCalculated == true)
            return;

        long oldHeight = height;
        int oldWidth = width;
        int oldLineStart = lineStart;
        
        lineHeight = charHeight;
        lineStart = charWidth * ((root.getLength()+"").length() + 3); // longest "hex" address can be (as a string), plus three (for a " : ")
        
        unitWidth = charWidth * 2; // length of two characters (eg FF).
        int asciiWidth = charWidth * hexPerLine; // how long the ascii stuff is

        int[] spaceEvery = new int[] {1, 2, 4, 8};
        int[] spaceSize = new int[]  {charWidth*3, 0, charWidth, charWidth };
        
        hexStart = new int[hexPerLine+1]; // where each hex starts on a line + where ascii starts
        
        int s = 0;
        for (int n=0; n<hexStart.length; n++) { // only place hexStart.length should be used. use hexPerLine instead.
            hexStart[n] = s;
            for (int m=0; m<spaceEvery.length; m++) {
                if ((n+1) % spaceEvery[m] == 0) {
                    s += spaceSize[m];
                }
            }
        }

        // if (root.isFixedLength()) {  // --- how to deal with non fixed length?
        
        totalLines = root.getLength() / hexPerLine;
        if ((float)totalLines != ((float)root.getLength() / hexPerLine))
            totalLines++;
        height = totalLines * lineHeight;
        width = lineStart + hexStart[hexPerLine] + unitWidth + asciiWidth;
        dimensionsCalculated = true;

        if (oldWidth != width || oldHeight != height || oldLineStart != lineStart) {
            revalidate();
        }
        //System.out.println("char: " + charWidth + "x" + charHeight + ". Dimensions: " + width + "x" + height +".");
    }
    
    public void paintComponent(Graphics g) {
        if (rootLength != root.getLength()) { // xxx: size change should be given via trigger
            reCalcDim();
        }
        Graphics2D g2 = (Graphics2D)g;
        super.paintComponent(g2);
	//System.out.println("paint called. " + g.getClipBounds());
	//System.out.println("this dimens.. " + this.getSize());
        if (dimensionsCalculated==false) {
            g2.setFont(font);            
            FontMetrics fm = g2.getFontMetrics();
            calcDim(fm);
        }
        int top, bot;
        Rectangle clipRect = g2.getClipBounds();
        if (clipRect != null) {
            //If it's more efficient, draw only the area specified by clipRect.
            
            /*
            // box around the just painted area
            g2.setColor(Color.pink);
            g2.drawRect(clipRect.x, clipRect.y, clipRect.x+clipRect.width-1, clipRect.y+clipRect.height-1);
            g2.setColor(Color.red);
            g2.drawLine(clipRect.x, clipRect.y, clipRect.x+clipRect.width-1, clipRect.y); // top
            g2.setColor(Color.green);
            g2.drawLine(clipRect.x, clipRect.y+clipRect.height-1, clipRect.x+clipRect.width-1, clipRect.y+clipRect.height-1); // bot
            */
            
            top = clipRect.y / lineHeight;
            bot = (clipRect.y + clipRect.height) / lineHeight + 1;

        } else {
            top = 0;
            bot = (int)totalLines; //XXX: precision loss!!
            //Paint the entire component.
            System.out.println("Warning: Whole hexpanel component got painted! that shouldn't really happen.");
        }
      
        paintLines(g2,top,bot);
    }

    /** paint each hex unit from start to finish (lines) */
    public void paintLines(Graphics2D g, long start, long finish) {
        //System.out.println("painting from " + start + "(" + start*hexPerLine + ") to " + finish + "(" + finish*hexPerLine + ")");
        g.setFont(font);
        FontMetrics fm = g.getFontMetrics();
        
        if (start < 0) {
            start = 0;
        }
        
        long len = root.getLength(); 

        long startByte = start*hexPerLine;
        long endByte = finish*hexPerLine;
        if (endByte > len)
            endByte = len;
        
        long linenum = start;
        long lastHex = finish*hexPerLine;
	long selStart = -1; 
	long selEnd = -1; 
        
        byte ba[] = new byte[1]; // current byte
        byte b;
        

        InputStream data = root.getDataStream(startByte,endByte-startByte);

	if (selection != null ) {
	    selStart = selection.getStart();
	    selEnd = selStart + selection.getLength();
	}
        
     
        for (long i = start*hexPerLine; i < len && i <=lastHex; i += hexPerLine) { // line (y)
	    StringBuffer ascii = new StringBuffer(16);
            
            // calc characters to draw on this line (jlen). Can't be moreo than hexPerLine.
            int jlen = (int)((len-i >= hexPerLine ? hexPerLine : len-i));
            for (int j=0; j < jlen; j++) { // unit (x)
                try {
                    data.read(ba); // read byte into b[]
                    b = ba[0];
                } catch (IOException e) {
                    //XXX
                    e.printStackTrace();
                    return;
                }
                
                // draw line of hex
                boolean selected;
                if (i+j >= selStart && i+j < selEnd) {
                    selected = true;
                } else {
                    selected = false;
                }
                if (selected) {
                    g.setColor( new Color(170,170,255) ); // xxx: cache
                    g.fillRect( lineStart + hexStart[j], (int)(lineHeight*linenum), hexStart[j+1]-hexStart[j], lineHeight);  //XXX: precision loss!
                    g.fillRect( lineStart + hexStart[j], (int)(lineHeight*linenum), hexStart[j+1]-hexStart[j], lineHeight);  //XXX: precision loss!
                }
                g.setColor(Color.black);
                g.drawString( byte2hex(b), lineStart + hexStart[j], (int)(charAscent + lineHeight*linenum)); //XXX: precision loss!
                //g.drawString( byte2hex(b), (float)lineStart + hexStart[j], (float)(charAscent + lineHeight*linenum)); //XXX: precision loss?
                
                if (greyMode == 1) {
                    int col = ~((int)b) & 0xff; // the "gamma" of grey squares
                    if (selected) {
                        g.setColor(new Color(col/3*2,col/3*2,col/2+128));
                    } else {
                        g.setColor(new Color(col,col,col));
                    }
                    g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth-1, lineHeight-1); //XXX: precision loss!
                    
                    //g.setColor(Color.red);
                    //g.drawString( byte2ascii(b), lineStart + hexStart[hexPerLine] + (charWidth*j), charAscent + lineHeight*linenum );
                } else if (greyMode == 2) {
                    int col = ~((int)b) & 0xf0; // the gamma of left/outer square
                    int col2 = (~((int)b) & 0x0f) << 4; // the gamma of right/inner square
                    col = col | (col >> 4); // turn a0 into aa
                    col2 = col2 | (col2 >> 4);
                    
                    if (selected) {
                        g.setColor(new Color(col/3*2,col/3*2,col/2+128));
                        g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth-1, lineHeight-1); //XXX: precision loss!
                        g.setColor(new Color(col2/3*2,col2/3*2,col2/2+128));
                        g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j) + (charWidth/2), (int)(lineHeight*linenum)+(lineHeight/2), (charWidth/2)-1, (lineHeight/2)-1); //XXX: precision loss!
                    } else {
                        g.setColor(new Color(col,col,col));
                        g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth-1, lineHeight-1); //XXX: precision loss!
                        g.setColor(new Color(col2,col2,col2));
                        g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j) + (charWidth/2), (int)(lineHeight*linenum)+(lineHeight/2), (charWidth/2)-1, (lineHeight/2)-1); //XXX: precision loss!
                    }
                    
                } else {
                    if (selected) {
                        g.setColor( new Color(170,170,255) ); // xxx: cache
                        g.fillRect(lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum), charWidth, lineHeight); //XXX: precision loss!
                    }
                    g.setColor( Color.black ); 
                    g.drawString( byte2ascii(b), lineStart + hexStart[hexPerLine] + (charWidth*j), (int)(lineHeight*linenum + charAscent) ); //XXX: precision loss!
                }
		//ascii.append(byte2ascii(b));
            }
	    g.setColor(Color.black);

            // draw address:
            String addr = BigInteger.valueOf(i).toString(16)+":  ";
            g.setColor(Color.darkGray);
            g.drawString( addr,lineStart - fm.stringWidth(addr),(int)(lineHeight*linenum + charAscent)); //XXX: precision loss!
            linenum++;
        }
    }
    
    public static String byte2ascii(byte b) {
	//if (b <= 0x20)
	    //return " ";
	
	//return Character.toString((char)b); //XXX: do this some other way?
        return (char)(((int)b) & 0xff) + "";

    }

    public void dataEdited(EditEvent e) {
    }
    
    public void dataLengthChanged(EditEvent e) {
    }
  
    public void fileSaved(FileEvent e) {
    }
    
    public void fileClosed(FileEvent e) {
        setOpenFile(new OpenFile(new EmptyData()));
    }
    public void selectionCopied(ClipboardEvent e) {
    }
    
    public void selectionMade(SelectionEvent e) {
        if (e.getSource() == this) {
            return;
        }
        
        setSelection(e.getTransparentData(), true);
    }
    
    public void selectionCleared(SelectionEvent e) {
        if (e.getSource() == this) {
            return;
        }
        
        clearSelection();
    }

    public void setGreyMode(int mode) {
        greyMode = mode;
        repaint();
        //XXXXXXXXXXX
    }

    public static String byte2hex(byte b) {
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return "" 
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
    
    public java.awt.Dimension getPreferredScrollableViewportSize() {
        return new Dimension(width, viewportHeight);
    }    
  
    public int getScrollableBlockIncrement(java.awt.Rectangle visibleRect, int orientation, int direction) {
        if (orientation == SwingConstants.VERTICAL) {
            return visibleRect.height - lineHeight;
        } else {
            return visibleRect.width - unitWidth;
        }
    }
    
    public boolean getScrollableTracksViewportHeight() {
        return false;
    }
    
    public boolean getScrollableTracksViewportWidth() {
        return false;
    }
    
    public int getScrollableUnitIncrement(java.awt.Rectangle visibleRect, int orientation, int direction) {
        if (orientation == SwingConstants.VERTICAL) {
            return lineHeight;
        } else {
            return unitWidth;
        }
    }
    
    public void dragEnter(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent) {
    }
    
    public void dragExit(java.awt.dnd.DropTargetEvent dropTargetEvent) {
    }
    
    public void dragOver(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent) {
    }
    
    public void drop(java.awt.dnd.DropTargetDropEvent dtde) {
        dtde.acceptDrop(dtde.getDropAction());
        System.out.println("accepted: " + dtde);
        
        Transferable t = dtde.getTransferable();
        System.out.println("transferable: " + t);
        System.out.println("local: " + dtde.isLocalTransfer());
        
        dtde.dropComplete(false);
    }
    
    public void dropActionChanged(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent) {
    }
    
}
