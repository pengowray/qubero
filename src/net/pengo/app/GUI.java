package net.pengo.app;
import java.awt.BorderLayout;
import java.awt.Color;
import java.awt.Container;
import java.awt.Dimension;
import java.awt.Font;
import java.awt.Graphics;
import java.awt.Image;
import java.awt.Toolkit;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;
import java.io.File;
import java.io.IOException;
import java.net.URL;

import javax.swing.JFileChooser;
import javax.swing.JFrame;
import javax.swing.JLabel;
import javax.swing.JScrollPane;
import javax.swing.JSplitPane;

import net.pengo.data.Data;
import net.pengo.data.DemoData;
import net.pengo.data.DiffData;
import net.pengo.data.EditableData;
import net.pengo.data.LargeFileData;
import net.pengo.hexdraw.original.HexPanel;
import net.pengo.hexdraw.original.renderer.Renderer;
import net.pengo.restree.SimpleResTree;
import net.pengo.splash.FontMetricsCache;

/** HexEditorGUI creates and coordinates all the graphical components. */
public class GUI implements ActiveFileListener {
    //protected OpenFile openFile; // the current nodedef being worked on
    
    private ActiveFile activeFile; // final
    
    protected JFrame jframe;
    protected HexPanel hexpanel;
    //protected HexTable hextable;
    protected JLabel statusbar;
    //protected ResTree moojtree;
    protected SimpleResTree simplemoojtree;
    protected MoojMenuBar mmb;
    protected Image icon;
    
    protected ExitAction exitAction;
    
    protected String titleSuffix =  "Qubero";
    protected String logo = "mooj32.png";
    
    public GUI() {
        setIcon();
        open(new DemoData(icon));
    }
    public GUI(ActiveFile af) {
        activeFile = af;
        activeFile.addActiveFileListener(this);
        open(af.getActive());
    }
    
    public GUI(Data d) {
        open(d);
    }
    
    public JFrame getJFrame() {
        return jframe;
    }
    /*
     public GUI(Data data)
     {
     open(data);
     }
     public GUI(EditableData data)
     {
     open(data);
     }
     public GUI(OpenFile openFile)
     {
     open(openFile);
     }
     */
    
    protected void setIcon() {
        if (icon != null)
            return;
        
        URL url = ClassLoader.getSystemResource(logo);
        if (url != null)
            icon = Toolkit.getDefaultToolkit().createImage(url);
    }
    
    protected void start() {
        if (jframe != null)
            return;
        
        //FIXME: in future use a renderer factory
        setIcon();
        
        // CREATE THE COMPONENTS
        //moojtree = ResTree.create(openFile);
        simplemoojtree = SimpleResTree.create(activeFile);
        
        hexpanel = new HexPanel(activeFile); //
        //hextable = new HexTable(activeFile);
        //LineRepeater hexpanel = new LineRepeater(openFile.getData());
        
        statusbar = new JLabel("Qubero: Vertical Modeller/Assembler");
        mmb = new MoojMenuBar(this);
        
        // ARRANGE THEM IN A FRAME
        jframe = new JFrame(activeFile + " - " + titleSuffix) {
            public void paint(Graphics g) {
                super.paint(g);
                FontMetricsCache.singleton().lendGraphics(g); //FIXME: only need to do this once?
            }
        };
        
        jframe.setIconImage(icon);
        jframe.setJMenuBar(mmb);
        
        //change it here!!!:
        JScrollPane sp_hexpanel = new JScrollPane(hexpanel, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS,
                JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        //JScrollPane sp_hexpanel = new JScrollPane(hextable, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS,
        //					  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        sp_hexpanel.getViewport().setBackground(Color.white); //FIXME: doesn't work!
        
        JScrollPane sp_moojtree = new JScrollPane(simplemoojtree, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED,
                JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        /*z
        JScrollPane sp_moojtree = new JScrollPane(moojtree, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED,
                JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        */
        JSplitPane splitpane = new JSplitPane(JSplitPane.HORIZONTAL_SPLIT, sp_moojtree, sp_hexpanel);
        //splitpane.setResizeWeight(1);
        //splitpane.setResizeWeight(.5);
        
        //jframe.setContentPane(sp_hexpanel);
        
        Container c = jframe.getContentPane();
        c.add(splitpane, BorderLayout.CENTER);
        c.add(statusbar, BorderLayout.SOUTH);
        
        jframe.pack();
        shrink(jframe);
        
        //splitpane.resetToPreferredSizes();
        int FRAMEBORDER = 15; //FIXME: cant work this out properly :/
        int treewidth = jframe.getWidth() -
        (FRAMEBORDER + sp_hexpanel.getVerticalScrollBar().getWidth() + hexpanel.getPreferredSize().width + splitpane.getDividerSize());
        //int treewidth = jframe.getWidth() -
        //(FRAMEBORDER + sp_hexpanel.getVerticalScrollBar().getWidth() + hexpanel.getPreferredSize().width + splitpane.getDividerSize());
        
        //int treewidth = c.getWidth() - hexpanel.getWidth();
        splitpane.setDividerLocation(treewidth);
        
        //check if window size is over the limit
        Dimension screen = Toolkit.getDefaultToolkit().getScreenSize();
        Dimension window = (Dimension)jframe.getSize().clone(); // is clone necessary? we are editing..
        int leftGap = jframe.getLocation().x; // gap between window and screen edge (i hope)
        int topGap = jframe.getLocation().y;
        
        boolean toowide = (window.width > screen.width);
        boolean toolong = (window.height > screen.height); 
        if (toowide || toolong ) {
            // get the normal size right first
            if (toowide)
                window.width = screen.width - (leftGap*2); // make gap to screen edge on either side even 
            if (toolong)
                window.height  = screen.height - (topGap*2);
            
            // then maximize
            jframe.setExtendedState(JFrame.MAXIMIZED_BOTH);
            
            //debug :)
            System.out.println("toowide:" + toowide + " toolong:" + toolong + " screensize:"+window.width+"x"+window.height + " screenPos:" + leftGap+","+topGap);
            
        }
        
        jframe.setVisible(true);
        
        // SET UP EVENTS
        jframe.addWindowListener(new WindowAdapter() {
            public void windowClosing(WindowEvent e) {
                quit();
            }
        });
        
        // DISPLAY
        
        
    }
    
    public void setStatusMessage(String msg) {
        statusbar.setText(msg);
    }
    
    public void openClass() {
        // to do
        
    }
    
    public void open(String filename) {
        open(new File(filename));
    }
    
    public void open(File file) {
        LargeFileData raw = new LargeFileData(file);
        open(raw);
    }
    
    public void open(Data raw) {
        if (raw instanceof EditableData) {
            EditableData eraw = (EditableData)raw;
            open(eraw);
            return;
        }
        DiffData draw = new DiffData(raw);
        OpenFile of = new OpenFile(draw);
        draw.setOpenFile(of); // just for break and mod res lists
        open(of);
    }
    
    public void open(EditableData raw) {
        open(new OpenFile(raw));
    }
    
    public void open(OpenFile of) {
        if (activeFile == null) {
            activeFile = new ActiveFile(of);
            activeFile.addActiveFileListener(this);
            start();
            activeFile.setActive(of, this);
        } else {
            activeFile.setActive(of, this);
        }
        
    }
    
    public void closeAll() {
        activeFile.closeAll(this); //FIXME: confirm close?!
        jframe.setTitle(titleSuffix);
    }
    
    public void closeActive() {
        activeFile.close(activeFile.getActive(), this); //FIXME: confirm close?!
        jframe.setTitle(titleSuffix);
    }
    
    public void saveAs() {
        try {
            JFileChooser chooser = new JFileChooser();
            int returnVal = chooser.showSaveDialog(jframe);
            if (returnVal == JFileChooser.APPROVE_OPTION) {
                //String filename = chooser.getSelectedFile().getName();
                activeFile.getActive().saveAs(this, chooser.getSelectedFile());
            }
        }
        catch (IOException e) {
            e.printStackTrace();
        }
    }
    
    public void quit() {
        //FIXME: Save changes?
        System.exit(0);
    }
    
    public void shrink(JFrame jframe) {
        //FIXME: shrink size if too big (hack!)
        int lowHeight = 50;
        int niceHeight = 500;
        int addWidth = 100; // give it an extra 100
        int height = jframe.getHeight();
        int width = jframe.getWidth();
        if (height > niceHeight) {
            jframe.setSize(width+addWidth,niceHeight);
        }
        else if (height < lowHeight) {
            jframe.setSize(width+addWidth,lowHeight);
        }
        else {
            jframe.setSize(width+addWidth,height);
        }
    }
    
    public Renderer[] getViewModes() {
        return hexpanel.getViewModes();
    }
    
    public Renderer getViewMode(String name) {
        return hexpanel.getViewMode(name);
    }
    
    
    /*
     
    public void setEditable(boolean editable) {
        if (editable) {
            Data oldData = openFile.getData();
            DiffData newData = new DiffData(openFile, oldData);
            OpenFile newOpenFile = new OpenFile(newData);
            newData.setOpenFile(newOpenFile);
            open(newOpenFile);
        } else {
            //FIXME: not reversible!
        }
    }
    */
    
    public Image getIcon() {
        return icon;
    }
    
    public void openFileRemoved(ActiveFileEvent e) {
        //do nothing
    }
    
    public void activeChanged(ActiveFileEvent e) {
        fixTitle();
    }
    public void openFileAdded(ActiveFileEvent e) {
        //do nothing
    }
    
    public boolean readyToCloseOpenFile(ActiveFileEvent e) {
        return true;
    }
    
    public void closedOpenFile(ActiveFileEvent e) {
        //fixTitle(); // unneeded?
    }
    
    public void openFileNameChanged(ActiveFileEvent e) {
        fixTitle();
    }
    
    private void fixTitle() {
        OpenFile of = activeFile.getActive();
        if (of == null) {
            jframe.setTitle(titleSuffix);
            return;
        }
        
        jframe.setTitle(of.toString() + " - " + titleSuffix);
    }
    
    public void fontSizeSmall() {
        Font bigger = FontMetricsCache.singleton().getFont("hex.S");
        hexpanel.setHexFont(bigger);
    }
    public void fontSizeMedium() {
        Font bigger = FontMetricsCache.singleton().getFont("hex");
        hexpanel.setHexFont(bigger);
    }
    
    public void fontSizeLarge() {
        Font bigger = FontMetricsCache.singleton().getFont("hex.L");
        hexpanel.setHexFont(bigger);
    }

    public void fontSizeXLarge() {
        Font bigger = FontMetricsCache.singleton().getFont("hex.XL");
        hexpanel.setHexFont(bigger);
    }
    
    public void setColumnCount(int count) {
        hexpanel.setColumnCount(count);
    }
}

