import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;
import java.io.*;
import java.net.*;

/** HexEditorGUI creates and coordinates all the graphical components. */
class HexEditorGUI {
    protected OpenFile openFile; // the current nodedef being worked on

    protected JFrame jframe;
    protected HexPanel hexpanel;
    protected JLabel statusbar;
    protected MoojTree moojtree;
    protected MoojMenuBar mmb;
    protected Image icon;

    protected ExitAction exitAction;
    
    protected String titleSuffix =  " - Mooj";

    public HexEditorGUI() {
        setIcon();        
        open(new DemoData(icon));
    }
    public HexEditorGUI(Data data) {
        open(data);
    }
    public HexEditorGUI(EditableData data) {
        open(data);
    }
    public HexEditorGUI(OpenFile openFile){
        open(openFile);
    }
    
    protected void setIcon() {
        if (icon != null)
            return;
        
        URL url = ClassLoader.getSystemResource("mooj32.png");
        if (url != null)
            icon = Toolkit.getDefaultToolkit().createImage(url);
    }

    protected void start() {
        //XXX: in future use a renderer factory
        setIcon();        
        
	// CREATE THE COMPONENTS
	moojtree = MoojTree.create(openFile);
	hexpanel = new HexPanel(openFile);
	statusbar = new JLabel("Mooj Data Modeller and Hex Editor");
	mmb = new MoojMenuBar(this);

	// ARRANGE THEM IN A FRAME
        jframe = new JFrame(openFile + titleSuffix );
        jframe.setIconImage(icon);
	jframe.setJMenuBar(mmb);
        JScrollPane sp_hexpanel = new JScrollPane(hexpanel, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS, 
						  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        sp_hexpanel.getViewport().setBackground(Color.white); //XXX: doesn't work!
        
        JScrollPane sp_moojtree = new JScrollPane(moojtree, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED, 
						  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
        
	JSplitPane splitpane = new JSplitPane(JSplitPane.HORIZONTAL_SPLIT, sp_moojtree, sp_hexpanel);
        splitpane.setResizeWeight(1);

        //jframe.setContentPane(sp_hexpanel);
        
        Container c = jframe.getContentPane();
     	c.add(splitpane, BorderLayout.CENTER);
        c.add(statusbar, BorderLayout.SOUTH);
        
        jframe.pack();
	shrink(jframe);
        
        //splitpane.resetToPreferredSizes();
        int FRAMEBORDER = 15; //XXX: cant work this out properly :/
        int treewidth = jframe.getWidth() - 
            (FRAMEBORDER + sp_hexpanel.getVerticalScrollBar().getWidth() + hexpanel.getPreferredSize().width + splitpane.getDividerSize());
        //int treewidth = c.getWidth() - hexpanel.getWidth();
        splitpane.setDividerLocation(treewidth);

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

    public void open() {
	JFileChooser chooser = new JFileChooser();
	int returnVal = chooser.showOpenDialog(jframe);
	if (returnVal == JFileChooser.APPROVE_OPTION) {
            //String filename = chooser.getSelectedFile().getName();
   	    open(chooser.getSelectedFile());
	}
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
        draw.setOpenFile(of);
        open(of);
    }

    public void open(EditableData raw) { 
        open(new OpenFile(raw));
    }
    
    public void open(OpenFile of) { 
        if (jframe == null) {
            this.openFile = of;
            start();
        } else {
            closeAll();
            this.openFile = of;
            hexpanel.setOpenFile(of);
            moojtree.addOpenFile(of);
            jframe.setTitle(openFile.toString() + titleSuffix);
        }
        
        //mmb.setOpenFile(of); // menu bar
    }
    
    public void closeAll() {
        openFile.close(this); //XXX: confirm close?!
        jframe.setTitle("Mooj");
    }

    public void saveAs() {
        try {
            JFileChooser chooser = new JFileChooser();
            int returnVal = chooser.showSaveDialog(jframe);
            if (returnVal == JFileChooser.APPROVE_OPTION) {
                //String filename = chooser.getSelectedFile().getName();
                openFile.saveAs(this, chooser.getSelectedFile());
            }
        } catch (IOException e) {
            e.printStackTrace();
        }
    }

    public void quit() {
	//XXX: Save changes?
	System.exit(0);
    }

    public void shrink(JFrame jframe) {
	//XXX: shrink size if too big (hack!)
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
	} else {
	    jframe.setSize(width+addWidth,height);
        }
    }
    
    public void setGreyMode(int mode) {
        hexpanel.setGreyMode(mode);
    }
    
    public void setEditable(boolean editable) {
        if (editable) {
            Data oldData = openFile.getData();
            DiffData newData = new DiffData(openFile, oldData);
            OpenFile newOpenFile = new OpenFile(newData);
            newData.setOpenFile(newOpenFile);
            open(newOpenFile);
        } else {
            //xxx: not reversible!
        }
    }
    
    public Image getIcon() {
        return icon;
    }
    
}

