package net.pengo.hexdraw.original;

/* this is the original hex panel. updated to work with the new selection model. */

import java.awt.Color;
import java.awt.Dimension;
import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Rectangle;
import java.awt.datatransfer.Transferable;
import java.awt.dnd.DropTargetListener;
import java.awt.event.MouseEvent;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;

import javax.swing.JPanel;
import javax.swing.Scrollable;
import javax.swing.SwingConstants;
import javax.swing.event.MouseInputAdapter;

import net.pengo.app.ActiveFile;
import net.pengo.app.ActiveFileEvent;
import net.pengo.app.ActiveFileListener;
import net.pengo.app.ClipboardEvent;
import net.pengo.app.FileEvent;
import net.pengo.app.OpenFile;
import net.pengo.app.OpenFileListener;
import net.pengo.app.SelectionEvent;
import net.pengo.data.Data;
import net.pengo.data.DataEvent;
import net.pengo.data.DataListener;
import net.pengo.data.EmptyData;
import net.pengo.hexdraw.original.renderer.AddressRenderer;
import net.pengo.hexdraw.original.renderer.AsciiRenderer;
import net.pengo.hexdraw.original.renderer.GreyScale2Renderer;
import net.pengo.hexdraw.original.renderer.GreyScaleRenderer;
import net.pengo.hexdraw.original.renderer.HexRenderer;
import net.pengo.hexdraw.original.renderer.RGBRenderer;
import net.pengo.hexdraw.original.renderer.RGBWave;
import net.pengo.hexdraw.original.renderer.Renderer;
import net.pengo.hexdraw.original.renderer.RendererListener;
import net.pengo.hexdraw.original.renderer.SeperatorRenderer;
import net.pengo.hexdraw.original.renderer.WaveRenderer;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.splash.FontMetricsCache;

//FIXME: openFile's resources should be considered in rendering ?

public class HexPanel extends JPanel implements DataListener, ActiveFileListener, OpenFileListener, Scrollable, LongListSelectionListener, DropTargetListener, RendererListener  {
    
    private ActiveFile activeFile = null;
    protected OpenFile openFile = null; // == activeFile.getActive()
    protected Data root;
    protected long rootLength; // cached value to check if it's changed size.
    
    protected boolean dimensionsCalculated = false;
    protected long height; // height of entire panel
    protected int width; // width of entire panel
    protected int viewportHeight = 500; // preferred viewport height
    
    //protected int lineStart; // where does the first character start... at 0!
    protected int hexPerLine = 16; // number of hex units shown per line
    
    //protected int[] hexStart; // where each hex starts on a line.
    protected int unitWidth; // length of two characters, basically. Used for H scroll distance
    
    protected boolean charSizeKnown = false;
    //protected int lineHeight; // height of one line of hex
    protected long totalLines;
    //protected int charWidth = 8, charHeight = 14; // laptop sizes
    //protected int charWidth = 7, charHeight = 16, charAscent = 12; // probable sizes(?)
    protected int charWidth, charHeight, charAscent; // charHeight is lineHeight
    
    //protected Data selection = null;
    //protected SegmentalLongListSelectionModel selectionModel = new SegmentalLongListSelectionModel();
    
    private boolean draggingMode = false; // mouse is currently dragging a selection
    //    private boolean published = false; // has the current selection been published?
    
    
    private Font font; // = new Font("Monospaced", Font.PLAIN, 11); // antialias?

	private boolean overwrite = true; // overwrite mode? (false for insert)
	Place cursor = null;
	
    Renderer renderers[];
    SeperatorRenderer seperator;
    
    private boolean[] checks;
    
    
    public HexPanel(ActiveFile activeFile)  {
        super();
        setBackground(Color.white);
        setupRenderers();
        
        this.activeFile = activeFile;
        activeFile.addActiveFileListener(this);
        setOpenFile(activeFile.getActive());
        setupMouse();

        calcDim(); // must be called AFTER setup renderers
        
    }
    
    public FontMetrics getFontMetrics() {
        if (font==null)
            font = FontMetricsCache.singleton().getFont("hex");
        
        return FontMetricsCache.singleton().getFontMetrics(font);
    }
    
    private void setupMouse(){
        // MOUSE ROUTINES
        
        addMouseListener(
                new MouseInputAdapter()  {
                    
                    public void mousePressed(MouseEvent e)  {
                        if (e.getButton() == MouseEvent.BUTTON3) {
                            // Right click!
                            openFile.liveSelection.getJMenu().getPopupMenu().show(HexPanel.this, e.getX(), e.getY());
                            //System.out.println("rclick ");
                            return;
                        }

                        Place pl = hexFromClick( e.getX(), e.getY() );
                        if (pl.addr != -1)  {
                            getSelectionModel().setValueIsAdjusting(true);
                            draggingMode = true;
                            
                            click(pl.addr, e);
                        }
                        else  {
                            draggingMode = false;
                            getSelectionModel().setValueIsAdjusting(false);
                        }
                        
                    }
                    
                    public synchronized void  mouseReleased(MouseEvent e)  {
                        if (draggingMode) // && selection != null
                        {
                            //System.out.println("release: " + e);
                            draggingMode = false;
                            getSelectionModel().setValueIsAdjusting(false);
                        }
                        
                    }
                    
                    public void mouseClicked(MouseEvent e)  {
                        int clicks = e.getClickCount();
                        if (clicks == 2) {
                            Place pl = hexFromClick( e.getX(), e.getY() );
                            if (pl.addr != -1)  {
                                startEdit(pl);
                            }
                            
                            //getSelectionModel().setValueIsAdjusting(false);
                            //draggingMode = false;
                        }
                    }
                    
                }
        );
        
        addMouseMotionListener(new MouseInputAdapter()  {
            public void mouseDragged(MouseEvent e)  {
                Place pl =  hexFromClick( e.getX(), e.getY() );
                if (draggingMode && pl.addr != -1)  {
                    //getSelectionModel().setLeadSelectionIndex(hclick);
                    changeSelection(pl.addr, false, true);
                }
                
            }
        });
        
    }
    
    private void setupRenderers(){
        FontMetrics fm = getFontMetrics();
        int columns = hexPerLine;
        
        seperator = new SeperatorRenderer(fm, columns, true);
        
        renderers = new Renderer[] {
                new AddressRenderer(fm, columns, false, 10), //base-10 address
                new AddressRenderer(fm, columns, false, 16),
                //new SeperatorRenderer(fm, columns, true),
                new HexRenderer(fm, columns, true),
                new HexRenderer(fm, columns, false, 10, false), // decimal (last false=no zeropad)
                new HexRenderer(fm, columns, false, 8, true), //octal 
                new HexRenderer(fm, columns, false, 2, true), //binary 
                new HexRenderer(fm, columns, false, 8, true, true, true), // c-flag
                new HexRenderer(fm, columns, false, 16, true, true, true), // c-flag but hex
                new AsciiRenderer(fm, columns, true, true), // ascii 
                new AsciiRenderer(fm, columns, false, false), // ascii (extended)
                //new SeperatorRenderer(fm, columns, true),
                new GreyScaleRenderer(fm, columns, false), // reenable
                //new SeperatorRenderer(fm, columns, false),
                new GreyScale2Renderer(fm, columns, false),
                //new SeperatorRenderer(fm, columns, true),
                new RGBRenderer(fm, columns, 2, false), // 2 channel
                new RGBRenderer(fm, columns, 3, false),
                new RGBRenderer(fm, columns, 4, false),
                new RGBRenderer(fm, columns, 5, false),
                new WaveRenderer(fm, columns, 8, true),  // 8 bit
                new WaveRenderer(fm, columns, 16, false), 
                new WaveRenderer(fm, columns, 24, false), 
                new WaveRenderer(fm, columns, 32, false), 
                new RGBWave(fm, columns, false, 3),
                new RGBWave(fm, columns, false, 4),
        };
        
        for (int i = 0; i < renderers.length; i++) {
            Renderer r = renderers[i];
            r.addRendererListener(this);
        }
    }
    
    public Renderer[] getEnabledRenderers() {
        // put spaces between renderers and only give back enabled ones too.
        ArrayList spacedRend = new ArrayList();
        
        for (int i = 0; i < renderers.length; i++) {
            Renderer r = renderers[i];
            if (r != null && r.isEnabled()) {
                spacedRend.add(r);
                if (seperator != null){
                    spacedRend.add(seperator);
                }
            }
        }
        
        return (Renderer[]) spacedRend.toArray(new Renderer[0]);
    }
    
    public void setColumnCount(int columns) {
        hexPerLine = columns;
        calcDim();
    }
    
    public void openFileNameChanged(ActiveFileEvent e) {
        // do nothing
    }
    
    public void openFileRemoved(ActiveFileEvent e) {
        // do nothing
    }
    
    public void openFileAdded(ActiveFileEvent e) {
        // do nothing
    }
    
    public void closedOpenFile(ActiveFileEvent e) {
        // do nothing
    }
    
    public boolean readyToCloseOpenFile(ActiveFileEvent e) {
        return true;
    }
    
    public void activeChanged(ActiveFileEvent e) {
        setOpenFile(activeFile.getActive());
    }
    
    private void startEdit(Place pl) {
        //Fixme: TODO
        System.out.println("now (not really) editing at: " + pl.addr + " with " + pl.r);
        
        //make a blinking cursor
        
        
    }
    
    // triggered when data changes
    public void dataUpdate(DataEvent e) {
        repaint();
    }
    
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     */
    public void valueChanged(LongListSelectionEvent e)  {
        
        //LongListSelectionModel sm = openFile.getSelectionModel();
        //repaintHex(sm.getMinSelectionIndex(), sm.getMaxSelectionIndex());
        repaint();
    }
    
    public void click(long hclick, MouseEvent e) {
        if (e.isShiftDown())  {
            //getSelectionModel().setLeadSelectionIndex(hclick);
            changeSelection(hclick, false, true);
        }
        else if (e.isControlDown())  {
            //getSelectionModel().addSelectionInterval(hclick,hclick);
            changeSelection(hclick, true, false);
        }
        else  {
            //getSelectionModel().setSelectionInterval(hclick,hclick);
            changeSelection(hclick, false, false);
        }
    }
    

    public void changeSelection(long index, boolean toggle, boolean extend)  {
        //super.changeSelection(rowIndex, columnIndex, toggle, extend);
        LongListSelectionModel sm  = getSelectionModel();
        boolean selected = sm.isSelectedIndex(index);
        
        if (extend)  {
            if (toggle)  {
                sm.setAnchorSelectionIndex(index);
            }
            else  {
                sm.setLeadSelectionIndex(index);
            }
        }
        else  {
            if (toggle)  {
                if (selected)  {
                    sm.removeSelectionInterval(index, index);
                }
                else  {
                    sm.addSelectionInterval(index, index);
                }
            }
            else  {
                sm.setSelectionInterval(index, index);
            }
        }
    }
    
    
    private void setOpenFile(OpenFile openFile)  {
	if (this.openFile != null)  {
	    this.openFile.removeOpenFileListener(this);
	    this.openFile.removeLongListSelectionListener(this);
	    this.openFile.removeDataListener(this);
	}
	
	this.openFile = openFile;
	if (openFile == null) {
	    this.root = new EmptyData();
	} else {
	    this.root = openFile.getData();
	    openFile.addOpenFileListener(this);
	    //openFile.addResourceListener(this);
	    openFile.addLongListSelectionListener(this);
	    openFile.addDataListener(this);
	}
	
	//getSelectionModel().clearSelection();
	long oldRootLength = rootLength; 
	rootLength = root.getLength(); // used to check if it's changed.. FIXME: should have listener
	if (rootLength != oldRootLength)
	    calcDim();
	
	long MAX_PREC = 16777216;
	if (height > Integer.MAX_VALUE)  {
	    System.out.println("Warning: Very long file being displayed. Display height of " + height + " > Integer.MAX_VALUE (" + Integer.MAX_VALUE + ")" );
	    System.out.println("      Size is " + (float)((double)height/Integer.MAX_VALUE)*100 + "% of Integer.MAX_VALUE" );
	    System.out.println("      Size is " + (float)((double)height/MAX_PREC)*100 + "% of Max precise display height" );
	}
	else if (height > MAX_PREC )  {
	    System.out.println("Note (for old versions of Java): Longish file being displayed. Max precise display height of " + MAX_PREC + " < actual display height of " + height + " < Integer.MAX_VALUE (" + Integer.MAX_VALUE + ")" );
	    System.out.println("      Size is " + (float)((double)height/Integer.MAX_VALUE)*100 + "% of Integer.MAX_VALUE" );
	    System.out.println("      Size is " + (float)((double)height/MAX_PREC)*100 + "% of Max precise display height" );
	}
	
    }
    
    

    
    public Place hexFromClick(int x, int y)  {
        
        int hx = -1;
        int hy = -1;

        // work out row (y)
        hy = y / charHeight;
        int rendererY = y - (hy*charHeight); // Y to give to pass to renderer
        
        // work out address of line (first hex)
        long startAddr = hy * hexPerLine; 
        long clickedOn = 0;
        
        int totalWidth = 0;
        Renderer r = null;
        Renderer[] rendererarray = getEnabledRenderers(); 
        for (int i = 0; i < rendererarray.length; i++) {
            r = rendererarray[i];
            if (r.isEnabled()) {
	            int width = r.getWidth();
	            int nextTotal = totalWidth+width;
	            
	            if (x >= totalWidth && x < nextTotal ){
	                //System.out.println("r"+ i + " x:" + (x - totalWidth) + " y:"+ rendererY + " start:" + startAddr);
	                clickedOn = r.whereAmI(x - totalWidth, rendererY, startAddr);
	                //System.out.println("clicked: " + clickedOn);
	                break;
	            }
	            
	            totalWidth = nextTotal;
            }
        }
        
        //dont let it go off the edge of the world
        if (clickedOn > rootLength)
            return new Place(hy, rootLength-1, r, overwrite);
        
        return new Place(hy, clickedOn, r, overwrite);
    }
    
    /*
     private void repaintAfterNewSelection(long newSelectionFirst, long newSelectionLast) {
     long minC, maxC; // min and max characters changed
     if (isOldSelection())
     {
     minC = Math.min(newSelectionFirst, oldSelectionFirst);
     maxC = Math.max(newSelectionLast, oldSelectionLast);
     }
     else
     {
     minC = newSelectionFirst;
     maxC = newSelectionLast;
     }
     
     repaintHex(minC, maxC);
     
     oldSelectionFirst = newSelectionFirst;
     oldSelectionLast = newSelectionLast;
     }
     */
    
    /** repaints between the two address positions.
     * e.g. repaintHex(0, 31) may repaint the first two lines
     */
    private void repaintHex(long minPos, long maxPos)  {
	//repaint only necessary areas
	//FIXME: this just gets converted back to hex units again, so why bother?
	//FIXME: doesn't trim X-ways
	
	//FIXME: loss of precision!
	repaint(0, (int)((minPos/hexPerLine)* charHeight), width, (int)(((maxPos/hexPerLine)+1)*(charHeight)));
	
    }
    
    /** returns a copy of the selection model */
    private LongListSelectionModel getSelectionModel()  {
	return openFile.getSelectionModel();
    }
    
    public Dimension getPreferredSize()  {
        //FIXME: this is a hack!
        return new Dimension(width, (int)height); //FIXME: precision loss!
    }
    
    public Dimension getMinimumSize()  {
        return getPreferredSize();
    }
    
    
    public Dimension getMaximumSize()  {
        return new Dimension(width, (int)height); //FIXME: precision loss!
    }
    
    //FIXME: will need def listener?
    
//    private void reCalcDim()  {
//        dimensionsCalculated = false;
//        calcDim();
//    }
    
    /** calculate dimensions */
    private void calcDim()  {
//        if (dimensionsCalculated == true)
//            return;
        
        //calcDim(getComponentGraphics().getFontMetrics(font)); // oops stupid no work
        FontMetrics fm = getFontMetrics();
        calcDim(fm);
    }
    
    private void calcDim(FontMetrics fm)  {
        
        //if (charSizeKnown == false)  {
            int oldCharWidth = charWidth;
            int oldCharHeight = charHeight;
            int oldCharAscent = charAscent;
            int oldWidth = width;
            long oldHeight = height;
            
            charWidth = fm.charWidth('W');
            charHeight = fm.getHeight();
            charAscent = fm.getAscent();
            unitWidth = charWidth * 2; // length of two characters (eg FF). -- used for H scroll distance
            
            totalLines = root.getLength() / hexPerLine;
            if ((float)totalLines != ((float)root.getLength() / hexPerLine))
                totalLines++;

            height = totalLines * charHeight;
            
            //calc width
            int totalWidth = 0;
            Renderer[] renderers = getEnabledRenderers();
            
            for (int index=0; index < renderers.length; index++) {	//render each renderer
                Renderer renderer = renderers[index];
                int segmentWidth = renderer.getWidth(); 
                totalWidth += segmentWidth;
            }
            width = totalWidth;
            
            /*
            charSizeKnown = true;
            if (oldCharWidth != charWidth || oldCharHeight != charHeight)  {
                System.out.println("Note: screen character size different to expected: " + charWidth  + "x" + charHeight + ",ascent:" + charAscent +  " instead of expected: " + oldCharWidth  + "x" + oldCharHeight + ",ascent:" + oldCharAscent);
            }
            if (oldCharAscent != charAscent )  {
                System.out.println("Note: character ascent different to expected: " + charAscent + " instead of expected: " + oldCharAscent);
            }
            */
            
            //dimensionsCalculated = true;
            //System.out.println("calc, height:"+height +" width:"+ width);
            
            if (oldWidth != width || oldHeight != height)  {
	            revalidate();
            }
        //}
        
    }
    
    public void paintComponent(Graphics g)  {
        if (rootLength != root.getLength()) { // FIXME: size change should be given via trigger
            calcDim();
        }
        
        Graphics2D g2 = (Graphics2D)g;
        super.paintComponent(g2);
        
        //System.out.println("paint called., width:"+width + ", height:"+height);
        //System.out.println("paint called. " + g.getClipBounds());
        //System.out.println("this dimens.. " + this.getSize());
        
//        if (dimensionsCalculated==false)  {
//            g2.setFont(font);
//            FontMetrics fm = g2.getFontMetrics();
//            calcDim(fm);
//        }
        int top, bot;
        Rectangle clipRect = g2.getClipBounds();
        if (clipRect != null)  {
            top = clipRect.y / charHeight;
            bot = (clipRect.y + clipRect.height) / charHeight + 1;
        }
        else  {
            top = 0;
            bot = (int)totalLines; //FIXME: precision loss!!
            //Paint the entire component.
            System.out.println("Warning: Whole hexpanel component got painted! that shouldn't really happen.");
        }
        
        paintLines(g2,top,bot);
    }
    
    /** paint each hex unit from start to finish (lines) */
    public void paintLines(Graphics g, long start, long finish)  {
        //System.out.println("painting from " + start + "(" + start*hexPerLine + ") to " + finish + "(" + finish*hexPerLine + ")");
        //System.out.println("rendering: "+start +"-"+finish);
        //g = g.create(0,0,width,charHeight);
        g.setFont(font);
        FontMetrics fm = g.getFontMetrics();
        
        if (start < 0)  {
            //fixme
            System.out.println("drawing from negative! (" + start +")");
            start = 0;
        }
        
        long startByte = start*hexPerLine;
        long endByte = finish*hexPerLine;
        
        long len = root.getLength();
        if (endByte > root.getLength())
            endByte = root.getLength();
        
        long offset = start;
        long lastHex = finish*hexPerLine;
        //long selStart = -1;
        //long selEnd = -1;
        

        boolean selecta[] = new boolean[hexPerLine]; // current selecteds
        byte ba[] = new byte[hexPerLine]; // current bytes
        
		int totalRenderWidth, segmentWidth; 

		int index, count;
	    Renderer renderer;
	    
	    //g.setClip( 0, (int)(start)*fm.getHeight(), width, (int)(finish)*fm.getHeight()+fm.getHeight() ); // possible problems with large i[ndex]
	    g.translate( 0,(int)(start)*fm.getHeight() );
	    
        try  {
            InputStream data = root.getDataStream(startByte,endByte-startByte);
            //FontMetrics fm = getFontMetrics();
            
            for (offset = start*hexPerLine; offset < len && offset <=lastHex; offset+=hexPerLine) { // line (y)
                // calc characters to draw on this line (jlen). Can't be moreo than hexPerLine.
                count = (int)((len-offset >= hexPerLine ? hexPerLine : len-offset));
				totalRenderWidth = 0;
 	

                try  {
                  //FIXME: data.readad(ba, 0, count) should work! but sometimes returning -1 
                  //count = data.read(ba, 0, count); // read bytes into ba[]
				  data.read(ba); // read bytes into ba[]
				  //count = data.read(ba); // read bytes into ba[]
                }
                catch (IOException e)  {
                    //FIXME:
                    e.printStackTrace();
                    return;
                }
                    
                // draw line of hex
                /*
                boolean selected =
                    openFile.getSelectionModel().isSelectedIndex(i+j) ||
                    (i+j >= activeSelectionFirst && i+j <= activeSelectionLast);
                */
                
                //renderByte( Point p1, Point p2, int index, byte b, boolena selected);
                
                
				//System.out.println("i:"+linenum +", lastHex:"+lastHex +", len:"+len +", count:"+count);
				//g.setClip( 0, (int)(linenum), width, charHeight ); // possible problems with large i[ndex]

				for ( index=0; index < selecta.length; index++) {	//setup selection
                	selecta[index] = openFile.getSelectionModel().isSelectedIndex((int)offset+index);
				}
				
				// get enabled renderers, and also seperator renderers between them
				Renderer[] renderers = getEnabledRenderers();
				
				for ( index=0; index < renderers.length; index++) {	//render each renderer
                    renderer = renderers[index];
					segmentWidth = renderer.getWidth(); 
					renderer.renderBytes( g, offset, ba, 0, count, selecta, cursor );
					totalRenderWidth += segmentWidth;
	                g.translate( segmentWidth, 0 );
				}
				//System.out.println("totalRenderWidth: " + totalRenderWidth +", charHeight:" + charHeight);
                g.translate( -totalRenderWidth, charHeight );
                //linenum++;
            }
        } catch (IOException e)  {
            //FIXME: now what?
			e.printStackTrace();
        }
    }
    

    
    public void dataEdited(DataEvent e)  {
    }
    
    public void dataLengthChanged(DataEvent e)  {
    }
    
    public void fileSaved(FileEvent e)  {
    }
    
    public void fileClosed(FileEvent e)  {
	setOpenFile(new OpenFile(new EmptyData()));
    }
    public void selectionCopied(ClipboardEvent e)  {
    }
    
    public void selectionMade(SelectionEvent e)  {
	if (e.getSource() == this)  {
	    return;
	}
	
	//setSelection(e.getSelectionResource(), true); //FIXME:
	repaint(); //FIXME: smaller area?
	
    }
    
    public void selectionCleared(SelectionEvent e)  {
	
    }
    public Renderer[] getViewModes() {
        return renderers;
    }
    
    public Renderer getViewMode(String name)  {
        for ( int i = 0; i < renderers.length; i++) {
            if (renderers[i] != null && renderers[i].toString().equals(name) )
                return renderers[i];
        }
        return null;
    }

/*
 	public void repaint()  {
		super.repaint();
	}
*/

    
    public java.awt.Dimension getPreferredScrollableViewportSize()  {
	return new Dimension(width, viewportHeight);
    }
    
    public int getScrollableBlockIncrement(java.awt.Rectangle visibleRect, int orientation, int direction)  {
	if (orientation == SwingConstants.VERTICAL)  {
	    return visibleRect.height - charHeight;
	}
	else  {
	    return visibleRect.width - unitWidth;
	}
    }
    
    public boolean getScrollableTracksViewportHeight()  {
	return false;
    }
    
    public boolean getScrollableTracksViewportWidth()  {
	return false;
    }
    
    public int getScrollableUnitIncrement(java.awt.Rectangle visibleRect, int orientation, int direction)  {
	if (orientation == SwingConstants.VERTICAL)  {
	    return charHeight;
	}
	else  {
	    return unitWidth;
	}
    }
    
    public void dragEnter(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent)  {
    }
    
    public void dragExit(java.awt.dnd.DropTargetEvent dropTargetEvent)  {
    }
    
    public void dragOver(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent)  {
    }
    
    public void drop(java.awt.dnd.DropTargetDropEvent dtde)  {
	dtde.acceptDrop(dtde.getDropAction());
	System.out.println("accepted: " + dtde);
	
	Transferable t = dtde.getTransferable();
	System.out.println("transferable: " + t);
	System.out.println("local: " + dtde.isLocalTransfer());
	
	dtde.dropComplete(false);
    }
    
    public void dropActionChanged(java.awt.dnd.DropTargetDragEvent dropTargetDragEvent)  {
        System.out.println("Drop targetted! " + dropTargetDragEvent);
    }


    public void rendererWidthUpdated() {
        calcDim();        
    }

    public void rendererDisplayUpdated() {
        repaint();
    }

    public void rendererEnabledUpdated(Renderer r) {
        calcDim();
    }
    
}
