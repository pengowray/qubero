/**
 * MethodSelection.java
 *
 * A way of selecting between multiple methods/classes of input for a properties page.
 * e.g. Combo box to select "Int" or "String" and pages for each type's input
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;
import java.awt.BorderLayout;
import javax.swing.*;
import java.awt.event.ActionListener;
import java.awt.event.ActionEvent;



public class MethodSelectionPage extends EditablePage {
    
    private PropertyPage[] page;
    private String name;
    protected int selected = 0;
    private JComboBox selectBox;
    
    private JPanel main;
    
    public MethodSelectionPage(PropertiesForm form, PropertyPage[] page, String name) {
        this(form, page, page, name);
    }
    
    public MethodSelectionPage(PropertiesForm form, PropertyPage[] page, Object[] pagenames, String name) {
        super(form);
        this.page = page;
        this.name = name;
        
        setLayout(new BorderLayout());
        
        selectBox = new JComboBox(pagenames);
        selectBox.addActionListener( new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                setNewSelected(MethodSelectionPage.this.selectBox.getSelectedIndex());
            }
        });
        setSelected(selected);
        
        main = new JPanel(new BorderLayout());
        main.add(selectBox, BorderLayout.NORTH);
        add(main, BorderLayout.CENTER);
        
        build();
    }
    
    public void setForm(PropertiesForm form) {
        super.setForm(form);
        for (int i = 0; i < page.length; i++) {
            page[i].setForm(form);
        }
    }
    
    public PropertyPage getSelected() {
        return page[selected];
    }
    
    // set the selection.. e.g. during building
    public void setSelected(int selected) {
        if (this.selected != selected) {
            main.remove(getSelected());
            this.selected = selected;
            build();
        }
    }
    
    // call when a user chooses a new selection
    public void setNewSelected(int selected) {
        if (this.selected != selected) {
            main.remove(getSelected());
            this.selected = selected;
            mod(); //fixme: bit of a hack.. AddressPage checks for mod before "fixing" the selection
            buildOp(); // can't just "build" because that resets modded (should it?)
            mod(); // need to mod() again so selected page thinks it's changed so it will save
        }
    }
    
    public void mod() {
        getSelected().mod();
        super.mod();
    }
    
    public void save() {
        // save regardless of if it's modded
        saveOp();
    }
    
    protected void saveOp() {
        //System.out.println("saving: " + getSelected());
        getSelected().save();
    }
    
    public void buildOp() {
        selectBox.setSelectedIndex(selected);
        getSelected().build();
        main.add(getSelected(), BorderLayout.CENTER);
        validate();
        repaint();
    }
    
    public String toString() {
        return name;
    }
    
    
    
}

