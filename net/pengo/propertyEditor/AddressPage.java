/*
 * AddressPage.java
 *
 * Created on July 12, 2003, 12:34 AM
 */

package net.pengo.propertyEditor;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.*;
/**
 *
 * @author  Smiley
 */


public class AddressPage extends EditablePage
{
    private IntResource res;
    private JTextField addressField;
    private JTextField lengthField;
    
    private ButtonGroup radioGroup;
    private JRadioButton simpleRadio;
    private JRadioButton multiRadio;
    private JRadioButton emptyRadio;
    
    /** Creates a new instance of AddressPage */
    public AddressPage(IntResource res, IntResourcePropertiesForm form)
	{
		super(form);
		this.res = res;
		
		//fixme: choose selection type via a drop down!
		JComboBox selectionType = new JComboBox(new String[] {"Simple Selecton","Multi-selection","Empty selection"});
		JComboBox sourceStart = new JComboBox(new String[] {"Fixed address","Variable address","Sequential"}); // ,"Recursive (tagged for now)","Dependent","layer","array"});
		JComboBox sourceLength = new JComboBox(new String[] {"Fixed length","Variable length"}); // ,"Until close tag","Dependent length","to end"});
		
		JPanel mainPanel = new JPanel(new GridLayout(0, 1));
		
		JPanel secondaryPanel = new JPanel();
		secondaryPanel.add(new JLabel("Selection type:"));
		secondaryPanel.add(selectionType);
		mainPanel.add(secondaryPanel);
		
		secondaryPanel = new JPanel();
		secondaryPanel.add(new JLabel("Start:"));
		secondaryPanel.add(sourceStart);
		mainPanel.add(secondaryPanel);
		
		secondaryPanel = new JPanel();
		secondaryPanel.add(new JLabel("Length:"));
		secondaryPanel.add(sourceLength);
		mainPanel.add(secondaryPanel);
		
		add(mainPanel);
		
		/*
		 addressField = new JTextField("0",12);
		 addressField.addActionListener( getSaveActionListener() );
		 addressField.getDocument().addDocumentListener( this );
		 lengthField = new JTextField("0",12);
		 lengthField.addActionListener( getSaveActionListener() );
		 lengthField.getDocument().addDocumentListener( this );
		 simpleRadio = new JRadioButton("Simple selection");
		 simpleRadio.addActionListener( getModActionListener() );
		 multiRadio = new JRadioButton("Multi-selection");
		 multiRadio.addActionListener( getModActionListener() );
		 emptyRadio = new JRadioButton("Empty selection"); //fixme: todo ??
		 emptyRadio.addActionListener( getModActionListener() );
		 radioGroup = new ButtonGroup();
		 radioGroup.add(simpleRadio);
		 radioGroup.add(multiRadio);
		 radioGroup.add(emptyRadio);
		 
		 JPanel radioPanel = new JPanel(new GridLayout(0, 1));
		 JPanel simplePanel1 = new JPanel();
		 JPanel simplePanel2 = new JPanel();
		 
		 radioPanel.add(simpleRadio);
		 
		 simplePanel1.add(new JLabel( "Address: " ));
		 simplePanel1.add(addressField);
		 simplePanel2.add(new JLabel( "Length: " ));
		 simplePanel2.add(lengthField);
		 radioPanel.add(simplePanel1);
		 radioPanel.add(simplePanel2);
		 radioPanel.add(multiRadio);
		 radioPanel.add(new JLabel("(multi-editor NYI)"));
		 radioPanel.add(emptyRadio);
		 add(radioPanel);
		 */
		build();
		
    }
    
    public void buildOp()
	{
		/*
		SelectionResource sel = res.getSelectionResource();
		SelectionData selData = sel.getSelectionData();
		addressField.setText(selData.getStart()+"");
		lengthField.setText(selData.getLength()+"");
		if (sel.getSelection().getSegmentCount() > 1)
		{
			multiRadio.setSelected(true);
		}
		else if (sel.getSelection().getSegmentCount() == 0)
		{
			addressField.setText("0");
			emptyRadio.setSelected(true);
		}
		else
		{
			simpleRadio.setSelected(true);
		}
		 */
    }
    
    public void saveOp()
	{
		if (simpleRadio.isSelected())
		{
			
		}
    }
    
    public String toString()
	{
		return "Address";
    }
}
