import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;

/* Minimal logic. View and Control only, preferably. But perhaps not at first. */
class HexEditorGUI {
    protected RawData root; // the data being edited
    protected DefaultMutableTreeNode def; // the definition of that data
    protected DefaultMutableTreeNode topnode; // the top node of the tree on the left, which the data goes under
    protected RawDataSelection currentSelection;
    protected DefaultMutableTreeNode currentSelectionNode = null;
    
    protected JTree tree;
    protected JLabel statusbar;
    protected JFrame jframe;

    public HexEditorGUI(RawData root){
        this.root = root;
        this.def = root.getDef();
        
        start();
    }

    protected void start() {
        //XXX: in future use a renderer factory

        jframe = new JFrame(root + " - Mooj" );
        
        
        
        HexPanelListener hpl = new HexPanelListener() {
            public void selectionMade(SelectionEvent e) {
                if (currentSelectionNode != null) {
                    int x = topnode.getIndex(currentSelectionNode);
                    if (x != -1) {
                        topnode.remove(x);
                    }
                }
                currentSelection = e.getSelection();
                currentSelectionNode = new DefaultMutableTreeNode(currentSelection,false);
                topnode.add(currentSelectionNode);
                jframe.getContentPane().remove(tree);
                tree = new JTree(topnode);
                jframe.getContentPane().add(tree, BorderLayout.WEST);
                statusbar.setText(".." + currentSelection);
            }
        };
    
        HexPanel hexpanel = new HexPanel(root,def,hpl);
        statusbar = hexpanel.getStatusbar();
        topnode = new DefaultMutableTreeNode("mooj",true);
        topnode.add(def);
      
        tree = new JTree(topnode);

        JScrollPane jsp = new JScrollPane(hexpanel, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS, JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        jframe.getContentPane().add(jsp, BorderLayout.CENTER);

        jframe.getContentPane().add(statusbar, BorderLayout.SOUTH);
        
        jframe.getContentPane().add(tree, BorderLayout.WEST);
        
        jframe.addWindowListener(new WindowAdapter() {
            public void windowClosing(WindowEvent e) {
                System.exit(0);
            }
        });
        
        jframe.pack();
        jframe.setVisible(true);
    }

    //protected void drawScreen(Canvas c) {
        // calc range to be displayed (int to int)
        // draw with spaces + preferences
        // allow clicking/selecting
    //}
}

interface HexPanelListener {
    public void selectionMade(SelectionEvent e);
}

class SelectionEvent {
    protected RawDataSelection sel;
    
    public SelectionEvent(RawDataSelection sel) {
        this.sel = sel;
    }
    
    public RawDataSelection getSelection() {
        return sel;
    }
}


class HexPanel extends JPanel {
    protected final RawData root;
    protected final TreeNode def;
    protected JLabel statusbar = new JLabel("Mooj Hex Editor"); // make more generic?
    protected HexPanelListener hexPanelListener;
    
    protected boolean dimensionsCalculated = false;
    protected int height = 600; // height of entire "sheet" of hex
    protected int width = 448; // width of entire "sheet" of hex
    protected int lineStart; // where does the first character start
    protected int hexPerLine = 32; // number of hex units shown per line
    protected int[] hexStart; // where each hex starts on a line.
    protected int unitWidth; // length of two characters, basically.
    protected int lineHeight; // height of one line of hex
    protected int totalLines;
    
    protected int selectionStart = -1;
    protected int selectionEnd = -1;
    protected int cursor = -1;
    
    Font font = new Font("Monospaced", Font.PLAIN, 11); // antialias?
    
    /**
     * To see the status bar, the hexpanel's container must getStatusbar and display it somewhere.
     */
    public JLabel getStatusbar() {
        //XXX: use a more generic interface than JLabel.
        //XXX: in future, a MDI container might use same statusbar for multiple HexEditors and prepend the file name to the message.
        return statusbar;
    }
    
    public void setStatusbar(JLabel statusbar) {
        this.statusbar = statusbar;
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
    
    /**
     * hpl may be null
     */
    public HexPanel(RawData root, TreeNode def, HexPanelListener hpl){
        //XXX: def should be used in rendering
        this.root = root;
        this.def = def;
        this.hexPanelListener = hpl;
        
        addMouseListener(
            new MouseInputAdapter() {
                public void mousePressed(MouseEvent e) {
                    mouseClicked(e);
                }
                public void mouseClicked(MouseEvent e) {
                    // e.getModifiers(); // FIXME: later.
                    int hclick = hexFromClick( e.getX(), e.getY() );
                    if (hclick != -1) {
                        selectionStart = hclick;
                        selectionEnd = hclick + 1;

                        statusbar.setText("selected: " + selectionStart + " to " + selectionEnd);
                        //XXX: this class should use selection class internally
                        hexPanelListener.selectionMade( new SelectionEvent( getSelection() ) );
                        repaint(); // fixme: repaint only necessary areas!
                    } else {
                        statusbar.setText("clicked on nothing");
                    }
                }
            }
        );

        addMouseMotionListener(new MouseInputAdapter() {
            public void mouseDragged(MouseEvent e) {
                int hclick = hexFromClick( e.getX(), e.getY() );
                if (hclick != -1) {
                    if (selectionStart == -1) {
                        selectionStart = hclick;
                    } else {
                        selectionEnd = hclick+1;
                    }
                    
                    //statusbar.setText("selected: " + selectionStart + " to " + selectionEnd);
                    hexPanelListener.selectionMade( new SelectionEvent( getSelection() ) );
                    repaint(); // fixme: repaint only necessary areas!
                    
                } else {
                    //statusbar.setText("dragged nowhere");    
                    //selectionStart = -1;
                    //selectionEnd = -1;
                }
        
            }
        });
        
    }
        
    public RawDataSelection getSelection() {
        return new RawDataSelection(root, selectionStart, selectionEnd-selectionStart);
    }

    public int getHeight() {
        return height;
   }
    
    public Dimension getPreferredSize() {
        //FIXME: this is a hack!
        return new Dimension(width, height);
    }

    public Dimension getSize() {
        return getPreferredSize();
    }
    
    //XXX: will need def listener?
 
    /** calculate dimensions */
    protected synchronized void calcDim(FontMetrics fm) {
        if (dimensionsCalculated == true)
            return;
        
        lineHeight = fm.getHeight();
        lineStart = fm.stringWidth("0000 0000    "); // ought to be enough space
        unitWidth = fm.stringWidth("WW"); // length of two characters, basically.
        
        int[] spaceEvery = new int[] {1, 2, 4, 8};
        int[] spaceSize = new int[]  {unitWidth*3/2, 0, unitWidth/2, unitWidth/2 };
        
        hexStart = new int[hexPerLine]; // where each hex starts on a line.
        
        int s = 0;
        for (int n=0; n<hexPerLine; n++) {
            hexStart[n] = s;
            for (int m=0; m<spaceEvery.length; m++) {
                if ((n+1) % spaceEvery[m] == 0) {
                    s += spaceSize[m];
                }
            }
        }

        hexPerLine = hexStart.length;
        
        
        // if (root.isFixedLength()) {  // --- how to deal with non fixed length?
        
        totalLines = root.getLength() / hexPerLine;
        if ((float)totalLines != ((float)root.getLength() / hexPerLine))
            totalLines++;
        height = totalLines * lineHeight; //FIXME: slight hack, adds one (two) to avoid rounding up. and long->int
         
        width = lineStart + hexStart[hexStart.length-1] + unitWidth;
        
        dimensionsCalculated = true;
        
        statusbar.setText("line height: " + lineHeight + ". unit width: " + unitWidth + ". width: " + width);
    }
    
    //public void paintComponent(Graphics g) {
    public void paint(Graphics g) {
        super.paint(g);
        if (dimensionsCalculated==false) {
            g.setFont(font);            
            FontMetrics fm = g.getFontMetrics();
            calcDim(fm);
        }
        
        int top, bot;
        Rectangle clipRect = g.getClipBounds();
        if (clipRect != null) {
            //If it's more efficient, draw only the area
            //specified by clipRect.
            //Top-leftmost point = (clipRect.x, clipRect.y)
            //Width, height = clipRect.width, clipRect.height
            //g.setColor(Color.white);
            //g.fillRect(clipRect.x, clipRect.y, clipRect.width, clipRect.height);
            
            top = clipRect.y / lineHeight;
            bot = (clipRect.y + clipRect.height) / lineHeight + 1;
        } else {
            top = 0;
            bot = totalLines;
            //Paint the entire component.
            
        }
      
        paintLines(g,top,bot);
    }

        /** paint each hex unit from start to finish */
    public void paintLines(Graphics g, int start, int finish) {
        //System.out.println("painting from " + start + "(" + start*hexPerLine + ") to " + finish + "(" + finish*hexPerLine + ")");
        
        g.setFont(font);
        FontMetrics fm = g.getFontMetrics();
        
        int len = root.getLength(); 
        byte[] data = root.getData();
        int linenum = start;
        int lastHex = finish*hexPerLine;
        for (int i = start*hexPerLine; i < len && i <=lastHex; i += hexPerLine) {
            linenum++;
            
            int jlen = (len-i >= hexPerLine ? hexPerLine : len-i);
            for (int j=0; j < jlen; j++) {
                // draw line of hex
                if (i+j >= selectionStart && i+j < selectionEnd) {
                    g.setColor(Color.magenta);
                } else {
                    g.setColor(Color.black);
                }
                g.drawString( byte2hex(data[i+j]), lineStart + hexStart[j], lineHeight*linenum);
                
            }
            //System.out.println(linenum + ": " + hexline);
            
            // draw address:
            String addr = i+":  ";
            g.setColor(Color.blue);
            g.drawString( addr,lineStart - fm.stringWidth(addr),lineHeight*linenum);
            
        }
                        
    }
    
    public static String byte2hex(byte b) {
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return "" 
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
}