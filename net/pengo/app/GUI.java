package net.pengo.app;
import java.awt.*;

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
import net.pengo.hexdraw.lineRepeater.LineRepeater;
import net.pengo.hexdraw.original.HexPanel;
import net.pengo.hexdraw.tabled.HexTable;
import net.pengo.restree.ResTree;
import net.pengo.splash.FontMetricsCache;

/** HexEditorGUI creates and coordinates all the graphical components. */
class GUI
{
    protected OpenFile openFile; // the current nodedef being worked on
	
    protected JFrame jframe;
    protected HexPanel hexpanel;
    protected HexTable hextable;
    protected JLabel statusbar;
    protected ResTree moojtree;
    protected MoojMenuBar mmb;
    protected Image icon;
	
    protected ExitAction exitAction;
    
    protected String titleSuffix =  " - Mooj";
	
    public GUI()
	{
		setIcon();
		open(new DemoData(icon));
    }
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
    
    protected void setIcon()
	{
		if (icon != null)
			return;
		
		URL url = ClassLoader.getSystemResource("mooj32.png");
		if (url != null)
			icon = Toolkit.getDefaultToolkit().createImage(url);
    }
	
    protected void start()
	{
		//FIXME: in future use a renderer factory
		setIcon();
		
		// CREATE THE COMPONENTS
		moojtree = ResTree.create(openFile);
		
		hexpanel = new HexPanel(openFile);
		//HexTable hexpanel = new HexTable(openFile);
		//LineRepeater hexpanel = new LineRepeater(openFile.getData());
		
		statusbar = new JLabel("Mooj Data Modeller and Hex Editor");
		mmb = new MoojMenuBar(this);
		
		// ARRANGE THEM IN A FRAME
		jframe = new JFrame(openFile + titleSuffix)
		{
			public void paint(Graphics g)
			{
				super.paint(g);
				FontMetricsCache.singleton().lendGraphics(g); //FIXME: only need to do this once?
			}
		};
		
		jframe.setIconImage(icon);
		jframe.setJMenuBar(mmb);
		JScrollPane sp_hexpanel = new JScrollPane(hexpanel, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS,
												  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
		//JScrollPane sp_hexpanel = new JScrollPane(hextable, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS,
		//					  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
		sp_hexpanel.getViewport().setBackground(Color.white); //FIXME: doesn't work!
		
		JScrollPane sp_moojtree = new JScrollPane(moojtree, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED,
												  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
		
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
		
		//int treewidth = c.getWidth() - hexpanel.getWidth();
		splitpane.setDividerLocation(treewidth);
		
		jframe.setVisible(true);
		
		// SET UP EVENTS
		jframe.addWindowListener(new WindowAdapter()
								 {
					public void windowClosing(WindowEvent e)
					{
						quit();
					}
				});
		
		// DISPLAY
		
		
    }
	
    public void setStatusMessage(String msg)
	{
		statusbar.setText(msg);
    }
	
    public void open()
	{
		JFileChooser chooser = new JFileChooser();
		int returnVal = chooser.showOpenDialog(jframe);
		if (returnVal == JFileChooser.APPROVE_OPTION)
		{
			//String filename = chooser.getSelectedFile().getName();
			open(chooser.getSelectedFile());
		}
    }
	
    public void open(String filename)
	{
		open(new File(filename));
    }
	
    public void open(File file)
	{
		LargeFileData raw = new LargeFileData(file);
		open(raw);
    }
	
    public void open(Data raw)
	{
		if (raw instanceof EditableData)
		{
			EditableData eraw = (EditableData)raw;
			open(eraw);
			return;
		}
		DiffData draw = new DiffData(raw);
		OpenFile of = new OpenFile(draw);
		draw.setOpenFile(of);
		open(of);
    }
	
    public void open(EditableData raw)
	{
		open(new OpenFile(raw));
    }
    
    public void open(OpenFile of)
	{
		if (jframe == null)
		{
			this.openFile = of;
			start();
		}
		else
		{
			closeAll();
			this.openFile = of;
			hexpanel.setOpenFile(of);
			moojtree.addOpenFile(of);
			jframe.setTitle(openFile.toString() + titleSuffix);
		}
		
		//mmb.setOpenFile(of); // menu bar
    }
    
    public void closeAll()
	{
		openFile.close(this); //FIXME: confirm close?!
		jframe.setTitle("Mooj");
    }
	
    public void saveAs()
	{
		try
		{
			JFileChooser chooser = new JFileChooser();
			int returnVal = chooser.showSaveDialog(jframe);
			if (returnVal == JFileChooser.APPROVE_OPTION)
			{
				//String filename = chooser.getSelectedFile().getName();
				openFile.saveAs(this, chooser.getSelectedFile());
			}
		}
		catch (IOException e)
		{
			e.printStackTrace();
		}
    }
	
    public void quit()
	{
		//FIXME: Save changes?
		System.exit(0);
    }
	
    public void shrink(JFrame jframe)
	{
		//FIXME: shrink size if too big (hack!)
		int lowHeight = 50;
		int niceHeight = 500;
		int addWidth = 100; // give it an extra 100
		int height = jframe.getHeight();
		int width = jframe.getWidth();
		if (height > niceHeight)
		{
			jframe.setSize(width+addWidth,niceHeight);
		}
		else if (height < lowHeight)
		{
			jframe.setSize(width+addWidth,lowHeight);
		}
		else
		{
			jframe.setSize(width+addWidth,height);
		}
    }
    
    public void setGreyMode(int mode)
	{
		hexpanel.setGreyMode(mode);
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
    
    public Image getIcon()
	{
		return icon;
    }
    
}

