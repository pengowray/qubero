/*
 * LinePanel.java
 *
 * Created on 11 September 2002, 10:02
 */

// attempt to create a custom way of redisplaying a panel (a line of hex) 
// without recreating it (i.e. just using one). didn't work. too fiddly.

import javax.swing.*;
import java.awt.*;
import java.util.*;

/**
 *
 * @author  pengo
 */
public class LineRepeater extends JPanel {
    protected Data datasource;
    
    protected LinePanel linePanel;
    protected int units = 16; // default units in a line
    protected int width;
    protected int height; //xxx: too short?
    
    public LineRepeater(Data datasource) {
        this.datasource = datasource;
        linePanel = new LinePanel();
        //linePanel.setLayout(new UnitLayoutManager()); //xxx: uncomment when i know it works? :)
        linePanel.setLayout(new FlowLayout());
        for (int i=0; i<units; i++) {
            linePanel.add(new HexUnit(i));
        }
        linePanel.setSize(linePanel.getPreferredSize()); //xxx: probably not neccessary ?
        System.out.println("line panel size: " + linePanel.getPreferredSize());
        System.out.println("line panel components: " + linePanel.getComponentCount());

        width = linePanel.getWidth();
        height = linePanel.getHeight() * (int)(datasource.getLength() / units);

        add(linePanel);
    }
    
    public void paintComponent(Graphics g) {
        super.paintComponent(g);
        Rectangle clip = g.getClipBounds();
        
        int lineHeight = linePanel.getHeight();
        int firstLine = clip.y / lineHeight;
        int lastLine = (clip.y + clip.height) / lineHeight +1; //xxx: +1 sometimes
        int dataShowStart = (clip.height / height) * units;
        int dataShowLength = (int)datasource.getLength(); //xxx: more than shown, but at least not more than the data. //xxx: loss of precision
        
        byte[] d = datasource.getTransparentData(dataShowStart,dataShowLength).getDataStreamAsArray();
        
        
        for (int line=firstLine; line < lastLine; line++) {
            Graphics gm = g.create(0, line*lineHeight, clip.width, lineHeight);
            linePanel.setLocation(0, line*units);
            linePanel.paint(gm, d, line*units); //xxx: will have sync problems. need multiple linePanels
        }
    }
    
    public Dimension getPreferredSize() {
        return new Dimension(width, height);
    }
    
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }
    
    public Dimension getMaximumSize() {
        // allow this to be bigger maybe?
        return getPreferredSize();
    }
}

class LinePanel extends JPanel {
    protected byte[] bytes; // currently drawing this.
    protected int offset;
    
    /** Creates a new instance of LinePanel */
    public LinePanel() {
        super();
        init();
    }
    public LinePanel(boolean isDoubleBuffered) {
        super(isDoubleBuffered);
        init();
    }
    public LinePanel(LayoutManager layout) {
        super(layout);
        init();
    }
    public LinePanel(LayoutManager layout, boolean isDoubleBuffered) {
        super(layout, isDoubleBuffered);
        init();
    }
    private void init() {
    }

    public void paint(Graphics g, byte[] bytes, int offset) {
        //xxx: flawed function. sync problems.
        this.bytes = bytes;
        this.offset = offset;
        paint(g);
    }
    
    public void paint(Graphics g) {
        super.paint(g);
        Rectangle r = g.getClipBounds();
        g.setColor(Color.BLUE);
        g.drawLine(r.x, r.y, r.x+r.width, r.y+r.height);
    }
    
    public byte getByte(int offset) {
        return bytes[this.offset + offset];
    }
    
    public UnitInfo getUnitInfo(int offset) {
        return new UnitInfo(getByte(offset), UnitInfo.NORMAL);
    }
        /** calculate size this would be painted at? */
    public Dimension paintSize(byte[] bytes, int offset) {
        return getSize();
    }
}

class UnitLayoutManager implements LayoutManager { //xxx: make LayoutManager2
    protected int unitStart[]; // length of array is "units"
    protected int unitBound[]; // start and end of unit, with spacing. length length is units+1.
    protected int maxHeight; // highest component
    
    public void addLayoutComponent(String name, Component comp) {
        // need to call layoutContainer ?
    }
    
    public void layoutContainer(Container parent) {
        calcBounds(parent, true);
    }
    
    public Dimension minimumLayoutSize(Container parent) {
        // should try squishing! (both components and padding/spacing.. give an option for which to do first or how much of each?)
        return preferredLayoutSize(parent);
    }
    
    public Dimension preferredLayoutSize(Container parent) {
        calcBounds(parent, false);
        return (new Dimension(unitBound[unitBound.length-1], maxHeight));
    }
    
    public void removeLayoutComponent(Component comp) {
    }
    
    // doLayout true to actually change the layout. false to just calculate where everything should be.
    // unitBound is currently ignored. (except for gross size) Should be used to create a nice box around each unit/component.
    protected void calcBounds(Container parent, boolean doLayout) {
        int units = parent.getComponentCount();
        //int unitWidth = (int)unitPainter.getPreferredSize().getWidth(); //xxx: precision loss. double->int
        int spacingWidth = FontMetricsCache.singleton().getFontMetrics("hex").charWidth('W'); // xxx: cache this
        //int spacingWidth = unitWidth;
        
        float align = .5f; // .5 == put half of each space to the left side of the unit
        int leftPad = spacingWidth; // how much to pad the left
        int rightPad = spacingWidth;

        if (units == 0) {
            unitStart = new int[] { leftPad } ;
            unitBound = new int[] { 0, leftPad + rightPad };
            maxHeight = 0;
            return;
        }
        
        unitStart = new int[units];
        unitBound = new int[units + 1]; // start and end of unit, with spacing
        
        int[] spaceEvery = new int[] {1, 2, 4, 8}; //xxx: use denominators
        
        int[] spaceSize = new int[] {spacingWidth, 0, spacingWidth, spacingWidth}; // original
        //int[] spaceSize = new int[] {spacingWidth/2, spacingWidth/2, spacingWidth, spacingWidth};
        
        
        int s = 0;
        unitBound[0] = 0;
        unitStart[0] = leftPad;
        Component c = parent.getComponent(0);
        if (doLayout) c.setSize(c.getPreferredSize());
        if (doLayout) c.setLocation(leftPad, 0);
        maxHeight = c.getHeight();
        
        for (int n=1; n<units; n++) { 
            //hexStart[n] = s;
                
            int gap = 0;
            for (int m=0; m<spaceEvery.length; m++) {
                if ((n+1) % spaceEvery[m] == 0) {
                    gap += spaceSize[m];
                }
            }
            Component c0 = parent.getComponent(n-1);
            Component c1 = parent.getComponent(n);
            
            unitStart[n] = unitStart[n-1] + c0.getPreferredSize().width + gap;
            unitBound[n] = unitStart[n-1] + c0.getPreferredSize().width + (int)(gap * align);
            
            if (doLayout) c1.setSize(c1.getPreferredSize());
            if (doLayout) c1.setLocation(unitStart[n], 0);
            
            maxHeight = Math.max(maxHeight, c.getHeight());
            
        }
        unitBound[units] = unitBound[units-1] + parent.getComponent(units-1).getWidth() + rightPad;
    }
    
}

//xxx: make flyweight
class UnitInfo {
    static final int NORMAL = 0;
    static final int EMPTY = 1;
    static final int SELECTED = 2;
    static final int EDITING = 3;
    static final int EDITING_PEER = 4; // another view of this byte is being edited

    protected byte b;
    protected int state;
    
    public UnitInfo(byte b, int state) {
        this.b = b;
        this.state = state;
    }
    public byte getByte(){
        return b;
    }
    public int getState() {
        return state;
    }    

    public String hexString() {
        //xxx: cache this / lazy init / table of 256 strings?
        
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return "" 
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
}

/** paint a single "unit". all components within the unit draw the same value. */
class UnitPainter extends Container {
    public void addImpl(Component comp, Object constraints, int index) {
        // check that component is a unit-ish component?
        // or wrap it in a UnitPainter 
        super.addImpl(comp,constraints,index);
    }
    
    public void paintComponents(Graphics g, UnitInfo unit) {
        Component[] com = getComponents();
        for (int i=0; i<com.length; i++) {
            if (com[i] instanceof HexUnit) { // xxx HexUnit or another class ???
                ((HexUnit)com[i]).paint(g, unit);
            } else {
                com[i].paint(g);
            }
        }
    }
}

class HexUnit extends Component {
    protected FontMetrics fm;
    protected Dimension prefSize;
    protected int offset; // used when asking parent for a unit.
    
    //xxx: getHowManyPrevAndForwardBytesNeededToDraw ?
    
    public HexUnit(int offset){
        super();
        this.offset = offset;
        setFont(FontMetricsCache.singleton().getFont("hex"));
        fm = FontMetricsCache.singleton().getFontMetrics("hex");
    }
    
    public void paint(Graphics g) {

        Container con = getParent();
        if (con instanceof LinePanel) {
            LinePanel linpan = (LinePanel)con;
            paint(g, linpan.getUnitInfo(offset));
        }
        // else do nothing

    
        ////
        Rectangle r = g.getClipBounds();
        g.setColor(Color.RED);
        g.drawLine(r.x, r.y, r.x+r.width, r.y+r.height);
        System.out.println("drawing " + offset);
        ////
    }

    public void paint(Graphics g, UnitInfo unit) {
        
        if (unit.getState() == unit.EMPTY)
            return;
        
        String str = unit.hexString();
        g.drawString(str, 0, fm.getAscent());
        System.out.println("drawing " + str); // xxx
    }
    
    public Dimension getPreferredSize() {
        if (prefSize == null) {
            prefSize = new Dimension(fm.getMaxAdvance() * 2, fm.getHeight());
        }
        return prefSize;
    }
    
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }
    
    public Dimension getMaximumSize() {
        // allow this to be bigger maybe?
        return getPreferredSize();
    }    
    
}